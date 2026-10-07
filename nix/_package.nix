# The build definition, shared by this flake's packages and by the overlay.
#
# Taking `pkgs` as arguments rather than closing over this flake's own is what lets the
# overlay build against a consumer's nixpkgs.
{
  lib,
  rustPlatform,
  makeBinaryWrapper,
  typst,
  cacert,
  ...
}:
rustPlatform.buildRustPackage {
  pname = "qr-server";
  # Read rather than repeated: a release bumps one place and this cannot fall behind it.
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  # Named explicitly rather than filtered, so target/ and .direnv/ stay out of the store
  # and an edit to the justfile does not rebuild the service.
  #
  # The migrations, the typst templates and the layout presets live under crates/, so
  # they arrive with the sources that embed them. .sqlx is the offline query cache;
  # without it SQLX_OFFLINE has nothing to read and every query! macro fails to expand.
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

  # There is no database in the sandbox, so the query! macros expand from the committed
  # cache. This is also what makes a stale .sqlx/ a build failure rather than a runtime
  # surprise.
  SQLX_OFFLINE = "true";

  # sqlx's default sslmode builds a TLS stack before it knows whether it will be used,
  # and the managed Postgres does serve TLS. Without a trust store that construction
  # fails rather than falling back.
  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";

  nativeBuildInputs = [ makeBinaryWrapper ];

  # Only the tests that need neither a database nor typst. The rest are #[ignore]d and
  # run by checks.integration, which starts a Postgres.
  doCheck = true;

  postInstall = ''
    # --prefix, not --suffix: the flake's typst must win over whatever is ambient, or a
    # sheet rendered on a laptop and one rendered in the pod come out of different
    # compilers. This is also what carries typst into the image's closure, so
    # nix/_image.nix does not list it.
    #
    # makeBinaryWrapper rather than makeWrapper, which keeps bash out of the image.
    #
    # The cache path is set because typst writes there only if a template ever imports a
    # @preview package, and /tmp is the pod's one writable directory.
    wrapProgram $out/bin/qr-server \
      --prefix PATH : ${lib.makeBinPath [ typst ]} \
      --set-default TYPST_PACKAGE_CACHE_PATH /tmp/typst-cache
  '';

  meta = {
    description = "Pre-printed QR stickers, and what they open";
    homepage = "https://qr.shine.town";
    mainProgram = "qr-server";
    license = with lib.licenses; [
      mit
      asl20
    ];
    platforms = lib.platforms.unix;
  };
}
