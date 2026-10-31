# The crate2nix instance, in a plain flavour and a linting one.
#
# Shared between the build and the lint check so the two cannot diverge. The linting
# flavour differs only in what it sets on the workspace's own crates, which is the whole
# point: every third-party dependency keeps the store path the plain flavour gave it, so
# linting costs the six crates here rather than the four hundred below them.
#
# `buildRustCrate` drives rustc directly rather than through cargo, so linting is a matter
# of swapping the driver for clippy-driver rather than of running `cargo clippy`.
{
  lib,
  pkgs,
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

  # Every crate that has (or later gains) a query! macro expands from the committed
  # cache. Applied below to the whole workspace rather than named one crate at a time:
  # a crate added tomorrow is covered without anyone remembering this file.
  sqlxEnv = _attrs: {
    SQLX_OFFLINE = "true";
    SQLX_OFFLINE_DIR = "${sqlxCache}/.sqlx";
    # The macros only fall back to CARGO when no cache file was found, but without it
    # they panic rather than explain that. Naming the toolchain's cargo keeps that
    # fallback on the same compiler as the build.
    CARGO = "${toolchain}/bin/cargo";
    # sqlx's default sslmode builds a TLS stack before it knows whether it will be used,
    # and the managed Postgres does serve TLS. Without a trust store that construction
    # fails rather than falling back.
    SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  };

  # Read off the directory rather than listed, so a crate added later is linted without
  # anyone remembering to add it here. `members = ["crates/*"]` makes the directory the
  # authority on this already.
  members = builtins.attrNames (builtins.readDir ../crates);

  # The package names are what crate2nix keys its overrides by. Directory names happen to
  # match here, but reading the manifest keeps that a coincidence rather than a rule.
  memberNames = map (
    dir: (lib.importTOML (../crates + "/${dir}/Cargo.toml")).package.name or dir
  ) members;

  sqlxOverrides = builtins.listToAttrs (
    map (name: {
      inherit name;
      value = sqlxEnv;
    }) memberNames
  );

  manifest = lib.importTOML ../Cargo.toml;

  # crate2nix does not emit rust-version, so CARGO_PKG_RUST_VERSION would come through
  # empty and clippy would lose the floor it suppresses suggestions against. Without it a
  # lint that cargo clippy holds back, because the API it suggests postdates the declared
  # minimum, fires here instead and the two disagree about the same source.
  msrv = manifest.workspace.package.rust-version;

  # `[lints] workspace = true` in a member manifest is resolved by cargo, not by crate2nix,
  # and crate2nix emits no lints at all. Nothing carries the table down to rustc unless it
  # is read here, so a check without this one is blind to exactly the rules the workspace
  # cared enough to write down.
  lints = manifest.workspace.lints or { };
in
{
  lint ? false,
}:
# `allFeatures` is bolted on beside crate2nix's own attributes rather than wrapped around
# them, so callers reach `workspaceMembers` exactly as the generated file offers it.
#
# It is here because crate2nix resolves a workspace member's features from its manifest
# and nothing else, while the suite these checks replace ran under `--all-features`. The
# difference is silent: qr-core's tests/store.rs is `#![cfg(feature = "testing")]` all the
# way down, and nothing else asks for that feature, so without this the file compiles to a
# test binary that reports zero tests and passes.
import ../Cargo.nix {
  inherit pkgs;
  # The toolchain from rust-toolchain.toml, not nixpkgs' rustc: the edition and the
  # lints this workspace is written against are the ones the devshell uses.
  buildRustCrateForPkgs =
    p:
    let
      base = p.buildRustCrate.override {
        rustc = toolchain;
        cargo = toolchain;
        # Only consulted when useClippy is set, so naming it here costs the plain
        # flavour nothing and keeps both flavours on one toolchain.
        clippy = toolchain;
        # Third-party crates keep upstream's overrides; every workspace member gets the
        # offline cache, whether or not it has a query! macro today.
        defaultCrateOverrides = p.defaultCrateOverrides // sqlxOverrides;
      };
    in
    crate:
    base (
      crate
      // lib.optionalAttrs (lint && builtins.elem crate.crateName memberNames) {
        useClippy = true;
        # The default cap of "allow" silences every lint including clippy's, which would
        # leave a check that passes because it looked at nothing.
        capLints = "forbid";
        rust-version = msrv;
        inherit lints;
        # Uncapping only lets the warnings through; this is what makes them fail.
        extraRustcOpts = (crate.extraRustcOpts or [ ]) ++ [
          "-D"
          "warnings"
        ];
      }
    );
}
// {
  # Everything `--all-features` would turn on for one workspace crate: its defaults plus
  # every feature its own manifest declares.
  allFeatures =
    crate:
    [ "default" ]
    ++ builtins.attrNames ((lib.importTOML (../crates + "/${crate}/Cargo.toml")).features or { });
}
