# Everything both devshells share. Not a flake-parts module (import-tree skips the
# underscore prefix) but a devshell submodule, imported by nix/devshell.nix and by the
# headed partition. They differ only in whether Playwright carries headed Chromium.
{
  config,
  pkgs,
  withChromium,
}:
let
  playwright = import ./_playwright.nix { inherit pkgs withChromium; };
in
{
  packages = [
    (pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml)
    config.treefmt.build.wrapper
    config.hk-nix.package
    pkgs.deadnix
    # buildRustPackage gets this from stdenv; a bare `cargo build` here needs a
    # linker named on PATH.
    pkgs.stdenv.cc
    pkgs.git
    pkgs.just

    # The same typst nix/_package.nix wraps the binary with, so a sheet rendered
    # by hand and a sheet rendered by the pod come out of one compiler.
    pkgs.typst

    # `just db-*`: initdb, pg_ctl, createdb, psql. The server rather than libpq,
    # because the local database is started from this shell.
    pkgs.postgresql

    # `just migrate` and `just sqlx-prepare`.
    pkgs.sqlx-cli

    # Pushing an image or a chart by hand, when CI is not the one doing it.
    pkgs.skopeo
    pkgs.kubernetes-helm
    pkgs.yq-go
    pkgs.jq

    playwright.playwright-test
  ];

  env = [
    {
      name = "RUST_BACKTRACE";
      value = "1";
    }
    # Build against the committed query cache, not against whatever DATABASE_URL
    # happens to point at.
    #
    # sqlx's macros prefer a live connection when DATABASE_URL is set, so without
    # this every cargo command needs `just db-up` first, and CI — which has no
    # Postgres on that address — fails to compile rather than to connect. Only
    # `just sqlx-prepare` turns it off, because regenerating the cache is the one
    # thing that genuinely needs a server.
    {
      name = "SQLX_OFFLINE";
      value = "true";
    }
    # Points at what `just db-up` starts. sqlx-cli and a bare `cargo run` read
    # this one variable.
    {
      name = "DATABASE_URL";
      value = "postgres://qr:qr@127.0.0.1:5433/qr";
    }
    # The binary reads QR_DATABASE_URL; sqlx-cli reads DATABASE_URL. Both are set
    # rather than one being made to understand the other.
    {
      name = "QR_DATABASE_URL";
      value = "postgres://qr:qr@127.0.0.1:5433/qr";
    }
    # Photo blobs. Production mounts a PVC here; locally it is a gitignored
    # directory, so `rm -rf` is the whole reset.
    {
      name = "QR_DATA_DIR";
      eval = "$PRJ_ROOT/.local/photos";
    }
    # What the QR codes encode. A sticker printed from a dev shell must not
    # resolve to production, so this differs from the deployed value.
    {
      name = "QR_PUBLIC_BASE_URL";
      value = "http://127.0.0.1:8080";
    }

    # The wrappers already point at this; naming it again keeps a bare `node`
    # and anything spawned from the tests pointing at the same store path.
    {
      name = "PLAYWRIGHT_BROWSERS_PATH";
      value = "${playwright.browsers}";
    }
    # The browsers come from the store, so Playwright's check for a
    # distribution it recognises has nothing useful to say.
    {
      name = "PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS";
      value = "true";
    }
  ];

  devshell.motd = "";
  devshell.startup.hk.text = config.hk-nix.shellHook;
}
