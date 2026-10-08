image_name := "git.shine.town/sshine/qr"
chart := "chart/qr"
# Under charts/ rather than beside the image: helm push appends the chart name, which is
# also the image name, so both would write tags into one OCI repository.
chart_registry := "oci://git.shine.town/sshine/charts"

pgdata := justfile_directory() / ".pgdata"
pgport := "5433"
migrations := "crates/qr-core/migrations"

_list:
    @just --list --unsorted

# Format every file treefmt knows
fmt:
    treefmt

# Check formatting without changing anything
fmt-check:
    treefmt --fail-on-change --no-cache

# Run clippy over everything
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Run the tests that need neither a database nor typst
test:
    cargo test --workspace --all-features

# Run every test, including the ones needing `just db-up` and typst
test-db:
    cargo test --workspace --all-features -- --include-ignored

# Build release
build:
    cargo build --release --all-features

# Generate documentation
doc *args='':
    cargo doc --no-deps --all-features {{ args }}

# Verify Cargo.lock is in step with the manifests
lock-check:
    cargo metadata --locked --format-version 1 > /dev/null

# This is the check nix/_package.nix performs by having no database at all; here it costs
# an incremental cargo check instead of a full build.

# Verify the committed .sqlx/ cache still expands every query! macro
sqlx-check:
    SQLX_OFFLINE=true cargo check --workspace --all-targets

# Run CI checks locally
ci: fmt-check lint sqlx-check test doc build lock-check
    @echo "All CI checks passed!"

# --- local database -----------------------------------------------------------

# A recipe rather than a devshell startup hook: a hook would start a server on every
# direnv reload and every subshell and leave it running after the terminal closed. Port
# 5433 rather than 5432 so a system Postgres can sit beside it.

# Start a Postgres in .pgdata/ on port 5433
db-up:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d "{{ pgdata }}/base" ]; then
      initdb --pgdata "{{ pgdata }}" --username postgres \
        --auth trust --encoding UTF8 --no-locale
    fi
    if ! pg_ctl --pgdata "{{ pgdata }}" status >/dev/null 2>&1; then
      pg_ctl --pgdata "{{ pgdata }}" --log "{{ pgdata }}/server.log" \
        --options "-p {{ pgport }} -k {{ pgdata }} -c listen_addresses=127.0.0.1" \
        --wait start
    fi
    psql -h 127.0.0.1 -p {{ pgport }} -U postgres -tAc \
      "select 1 from pg_roles where rolname = 'qr'" | grep -q 1 \
      || createuser -h 127.0.0.1 -p {{ pgport }} -U postgres --createdb qr
    psql -h 127.0.0.1 -p {{ pgport }} -U postgres -tAc \
      "select 1 from pg_database where datname = 'qr'" | grep -q 1 \
      || createdb -h 127.0.0.1 -p {{ pgport }} -U postgres --owner qr qr
    psql -h 127.0.0.1 -p {{ pgport }} -U postgres -q -c \
      "alter role qr with password 'qr'"
    echo "postgres up on {{ pgport }}; DATABASE_URL is already set in the devshell"

# Stop the local Postgres
db-down:
    pg_ctl --pgdata "{{ pgdata }}" --mode fast --wait stop

# Throw the local database away and rebuild it from the migrations
db-reset:
    -just db-down
    rm -rf "{{ pgdata }}"
    just db-up
    just migrate

# Open a psql against the local database
psql *args:
    psql "$DATABASE_URL" {{ args }}

# Apply the migrations to $DATABASE_URL
migrate:
    sqlx migrate run --source {{ migrations }}

# Add a migration: `just migration add-photo-caption`
migration name:
    sqlx migrate add --source {{ migrations }} {{ name }}

# Regenerate the committed .sqlx/ offline query cache (needs `just db-up`)
sqlx-prepare: migrate
    cargo sqlx prepare --workspace -- --all-targets

# --- running ------------------------------------------------------------------

# Run the service against the local database
run *args:
    cargo run -p qr-server -- serve {{ args }}

# Render a sheet of codes to a PDF
sheet count='21' preset='relief-t7160' *args:
    cargo run -p qr-server -- sheet --count {{ count }} --preset {{ preset }} {{ args }}

# Print a calibration sheet to hold against real label stock
calibrate preset='relief-t7160':
    cargo run -p qr-server -- sheet --calibration --preset {{ preset }} --out calibration.pdf

# Show what each code length and error-correction level costs in QR modules
budget *args:
    cargo run -p qr-server -- budget {{ args }}

# --- image --------------------------------------------------------------------

# Build the OCI image (a script that streams a tarball on stdout)
image:
    nix build .#image -o image

# Build the image and load it into the local docker daemon
image-load: image
    ./image | docker load

# SKOPEO_DEST_CREDS ("user:token") is consumed when set; absent, skopeo falls back to
# ambient auth from ~/.config/containers/auth.json.

# Push the image as :latest, :<short-sha>, and :<tag> when on a tag
image-push: image
    #!/usr/bin/env bash
    set -euo pipefail
    args=()
    if [ -n "${SKOPEO_DEST_CREDS:-}" ]; then
      args=(--dest-creds "$SKOPEO_DEST_CREDS")
    fi
    tags="latest $(git rev-parse --short HEAD)"
    if git describe --exact-match --tags HEAD >/dev/null 2>&1; then
      tags="$tags $(git describe --exact-match --tags HEAD | sed 's/^v//')"
    fi
    # streamLayeredImage writes the tarball to stdout rather than the store, so this
    # re-runs the stream per tag instead of materialising a few hundred MB.
    for tag in $tags; do
      echo "pushing {{ image_name }}:$tag"
      ./image | skopeo copy --insecure-policy "${args[@]}" \
        docker-archive:/dev/stdin "docker://{{ image_name }}:$tag"
    done

# --- chart --------------------------------------------------------------------

# Lint the chart, and fail on any object that renders without a namespace
chart-lint:
    #!/usr/bin/env bash
    set -euo pipefail
    helm lint {{ chart }} --set postgres.existingSecret=lint
    helm template lint-release {{ chart }} --set postgres.existingSecret=lint > /dev/null
    # Every object must state its namespace. `helm install` supplies one, but kubenix
    # renders the chart and applies the manifest itself, so an object without one is
    # created wherever kubectl happens to point. Every optional template is switched on
    # here, because the one that regresses is the one nothing renders by default.
    missing=$(helm template ns-check {{ chart }} --namespace ns-check \
      --set postgres.existingSecret=lint \
      --set serviceAccount.create=true \
      --set serviceMonitor.enabled=true \
      | yq --no-doc 'select(.metadata.namespace == null) | .kind + "/" + .metadata.name')
    if [ -n "$missing" ]; then
      echo "these objects render without a namespace:" >&2
      echo "$missing" >&2
      exit 1
    fi

# The chart and the binary are separate artifacts with no compiler between them, so
# without this a renamed flag ships and is discovered as a CrashLoopBackOff.

# Feed the chart's rendered arguments to the binary, and fail if it rejects them
chart-args-check:
    #!/usr/bin/env bash
    set -euo pipefail
    workdir=$(mktemp -d)
    trap 'rm -rf "$workdir"' EXIT

    # nix build rather than cargo build: the release job builds this same derivation for
    # the image, so here it is a store hit.
    binary="$(nix build .#default --no-link --print-out-paths)/bin/qr-server"

    helm template check {{ chart }} --namespace check \
      --set postgres.existingSecret=check > "$workdir/rendered.yaml"

    for kind in containers initContainers; do
      mapfile -t argv < <(yq --no-doc \
        "select(.kind == \"Deployment\") | .spec.template.spec.$kind[0] | (.command[], .args[])" \
        "$workdir/rendered.yaml")
      [ "${#argv[@]}" -gt 0 ] || { echo "no $kind args rendered" >&2; exit 1; }
      printf '%s renders:\n' "$kind"
      printf '  %s\n' "${argv[@]}"
      # The first element is the binary path inside the image; drop it and hand the rest
      # to the binary that is actually here.
      "$binary" "${argv[@]:1}" --check-config
    done

# Package the chart into dist/
chart-package: chart-lint chart-args-check
    rm -rf dist && mkdir -p dist
    helm package {{ chart }} --destination dist

# Push the chart to the OCI registry (HELM_REGISTRY_USER/PASSWORD in CI)
chart-push: chart-package
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${HELM_REGISTRY_PASSWORD:-}" ]; then
      echo "$HELM_REGISTRY_PASSWORD" | helm registry login git.shine.town \
        --username "${HELM_REGISTRY_USER:-forgejo_admin}" --password-stdin
    fi
    helm push dist/*.tgz {{ chart_registry }}

# Clean build artifacts
clean:
    cargo clean
    rm -rf dist image image.tar.gz
