# Clippy over the whole workspace, as a check rather than a devshell command.
#
# One derivation per crate, through the same crate2nix instance the binary is built with,
# rather than one `cargo clippy` over a vendored workspace. Dependencies are the store
# paths the build already made, so this costs the six crates in `crates/` and nothing
# underneath them.
#
# `runTests` is what reaches the test targets: it wires the dev-dependencies in and
# compiles `tests/*.rs` alongside the lib, which is what `--all-targets` means to the
# devshell's `just lint`. The binaries are asked to list their tests rather than run them
# — running them is nix/_integration.nix's job, and a lint check that needed a database
# would be a lint check nobody could run.
{
  lib,
  callPackage,
  runCommand,
  ...
}:
let
  cargoNix = callPackage ./_cargo-nix.nix { } { lint = true; };

  linted = lib.mapAttrsToList (
    name: member:
    member.build.override {
      features = cargoNix.allFeatures name;
      runTests = true;
      testCrateFlags = [ "--list" ];
    }
  ) cargoNix.workspaceMembers;
in
runCommand "qr-clippy"
  {
    # Naming them here is what makes them build; nothing is read back out.
    inherit linted;
    meta.description = "Clippy over the qr workspace";
  }
  ''
    for crate in $linted; do
      echo "linted $crate"
    done
    touch $out
  ''
