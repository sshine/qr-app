# The tests that need a database and a typst, run inside the Nix sandbox.
#
# Those tests are #[ignore]d rather than feature-gated, so `--all-features` cannot switch
# them on where there is no server; this is the derivation that switches them on
# deliberately. It is a second build of the same sources rather than a flag on the
# package, so an ordinary `nix build` stays fast and a release does not wait on Postgres.
#
# Postgres listens on loopback rather than a unix socket. The sandbox has no network but
# it does have a loopback interface, and each build gets a namespace of its own, so a
# fixed port cannot collide with another build.
{
  lib,
  rustPlatform,
  postgresql,
  typst,
  cacert,
  ...
}:
let
  port = "5433";
in
rustPlatform.buildRustPackage {
  pname = "qr-integration";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../.sqlx
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  SQLX_OFFLINE = "true";
  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";

  # Ten statically linked test binaries, each carrying the whole dependency tree, is the
  # largest thing this flake asks a runner to hold. Full debug info doubles it.
  #
  # Line tables rather than none: what a CI failure is worth is the file and line a
  # backtrace names, and that survives. What goes is the type and variable detail nobody
  # reads without a debugger attached, which there is no way to attach here.
  CARGO_INCREMENTAL = "0";
  CARGO_PROFILE_DEV_DEBUG = "line-tables-only";
  CARGO_PROFILE_DEV_BUILD_OVERRIDE_DEBUG = "false";

  nativeBuildInputs = [
    postgresql
    typst
  ];

  # Nothing is installed; this derivation exists for its check phase.
  buildPhase = "runHook preBuild; runHook postBuild";
  installPhase = "runHook preInstall; touch $out; runHook postInstall";

  doCheck = true;
  checkPhase = ''
    runHook preCheck

    export PGDATA="$TMPDIR/pgdata"
    initdb --username postgres --auth trust --encoding UTF8 --no-locale >/dev/null
    pg_ctl --pgdata "$PGDATA" --log "$TMPDIR/pg.log" \
      --options "-p ${port} -c listen_addresses=127.0.0.1 -k $TMPDIR" \
      --wait start

    export QR_TEST_DATABASE_URL="postgres://postgres@127.0.0.1:${port}/postgres"
    export QR_TYPST_BIN="${lib.getExe typst}"

    cargo test --workspace --all-features --no-fail-fast -- --include-ignored

    pg_ctl --pgdata "$PGDATA" --mode immediate --wait stop

    runHook postCheck
  '';

  meta.description = "The qr tests that need a database and a typst";
}
