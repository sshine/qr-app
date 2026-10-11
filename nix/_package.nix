# The build definition, shared by this flake's packages and by the overlay.
#
# Taking `pkgs` as arguments rather than closing over this flake's own is what lets the
# overlay build against a consumer's nixpkgs.
#
# Built through crate2nix rather than buildRustPackage: one derivation per crate instead
# of one for the whole workspace, so an unchanged dependency is a store hit rather than a
# recompile. Cargo.nix is generated and committed; `just crate2nix` regenerates it and
# `just crate2nix-check` is what notices when it has fallen behind the manifests.
{
  lib,
  pkgs,
  stdenv,
  makeBinaryWrapper,
  typst,
  cacert,
  rust-bin,
  ...
}:
let
  toolchain = rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;

  # The committed query cache, as a store path of its own.
  #
  # crate2nix gives every crate its own directory as `src`, so the workspace root — and
  # with it `.sqlx` — is not in the sandbox the way buildRustPackage's whole-workspace
  # source was. SQLX_OFFLINE_DIR is what points the macros back at it; without it every
  # query! fails to expand and the error names a database rather than this.
  sqlxCache = lib.fileset.toSource {
    root = ../.;
    fileset = ../.sqlx;
  };

  # Every crate whose query! macros expand from the committed cache.
  sqlxEnv = _attrs: {
    SQLX_OFFLINE = "true";
    SQLX_OFFLINE_DIR = "${sqlxCache}/.sqlx";
    # sqlx's default sslmode builds a TLS stack before it knows whether it will be used,
    # and the managed Postgres does serve TLS. Without a trust store that construction
    # fails rather than falling back.
    SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  };

  cargoNix = import ../Cargo.nix {
    inherit pkgs;
    # The toolchain from rust-toolchain.toml, not nixpkgs' rustc: the edition and the
    # lints this workspace is written against are the ones the devshell uses.
    buildRustCrateForPkgs =
      p:
      p.buildRustCrate.override {
        rustc = toolchain;
        cargo = toolchain;
        defaultCrateOverrides = p.defaultCrateOverrides // {
          qr-core = sqlxEnv;
          qr-forward = sqlxEnv;
          qr-photos = sqlxEnv;
        };
      };
  };

  unwrapped = cargoNix.workspaceMembers.qr-server.build;
in
stdenv.mkDerivation {
  pname = "qr-server";
  # Read rather than repeated: a release bumps one place and this cannot fall behind it.
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  dontUnpack = true;
  nativeBuildInputs = [ makeBinaryWrapper ];

  # --prefix, not --suffix: the flake's typst must win over whatever is ambient, or a
  # sheet rendered on a laptop and one rendered in the pod come out of different
  # compilers. This is also what carries typst into the image's closure, so
  # nix/_image.nix does not list it.
  #
  # makeBinaryWrapper rather than makeWrapper, which keeps bash out of the image.
  #
  # The cache path is set because typst writes there only if a template ever imports a
  # @preview package, and /tmp is the pod's one writable directory.
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    makeBinaryWrapper ${unwrapped}/bin/qr-server $out/bin/qr-server \
      --prefix PATH : ${lib.makeBinPath [ typst ]} \
      --set-default TYPST_PACKAGE_CACHE_PATH /tmp/typst-cache
    runHook postInstall
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
