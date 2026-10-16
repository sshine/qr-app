# Clippy over the whole workspace, as a check rather than a devshell command.
#
# A derivation so `nix flake check` covers it and the result is cached: CI otherwise
# builds the workspace once for clippy, once for the tests and once for the release
# binary, three times over, on a runner that has other work to do.
#
# Clippy comes from the toolchain file rather than nixpkgs, so the lints CI enforces are
# the lints the devshell's `just lint` enforces.
{
  lib,
  rustPlatform,
  rust-bin,
  cacert,
  ...
}:
let
  toolchain = rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
in
rustPlatform.buildRustPackage {
  pname = "qr-clippy";
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

  # A lint pass has no use for any of this, and all of it is disk the runner has to find.
  #
  # The incremental cache is written to be read by the next build, and in a sandbox there
  # is never a next build. Debug info is a third of what is left, in a derivation whose
  # output is an empty file: nothing here is ever run, stepped through or backtraced. The
  # build-override line is separate because build scripts are compiled under their own
  # profile and keep their debug info when the main one drops it.
  CARGO_INCREMENTAL = "0";
  CARGO_PROFILE_DEV_DEBUG = "none";
  CARGO_PROFILE_DEV_BUILD_OVERRIDE_DEBUG = "false";

  # Ahead of the stdenv's rustc, so clippy-driver and the compiler it drives are one
  # toolchain. A mismatch between them fails with a metadata version error that reads
  # like a broken build rather than a wrong PATH.
  nativeBuildInputs = [ toolchain ];

  buildPhase = ''
    runHook preBuild
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    runHook postBuild
  '';

  doCheck = false;
  installPhase = "runHook preInstall; touch $out; runHook postInstall";

  meta.description = "Clippy over the qr workspace";
}
