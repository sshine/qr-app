{ inputs, ... }:
{
  imports = [ inputs.hk-nix.flakeModules.default ];
  perSystem =
    {
      config,
      pkgs,
      lib,
      system,
      ...
    }:
    let
      # By absolute store path: the `nix flake check` hk-check sandbox runs hooks
      # without the devshell PATH, so a relative `treefmt` is not found.
      treefmt = lib.getExe config.treefmt.build.wrapper;
      betterleaks = lib.getExe pkgs.betterleaks;
      crate2nix = lib.getExe inputs.crate2nix.packages.${system}.default;
    in
    {
      hk-nix.settings.hooks = {
        "pre-commit" = {
          fix = true;
          stash = "git";
          steps = {
            treefmt.check = "${treefmt} --fail-on-change --no-cache {{files}}";
            treefmt.fix = "${treefmt} {{files}}";
            betterleaks.check = "${betterleaks} dir --redact --no-banner --config .betterleaks.toml {{files}}";
          };
        };

        "pre-push".steps = {
          # Cargo.nix is 15k lines of crate2nix output and carries unused arguments
          # throughout; it is regenerated, never edited, so there is nothing to act on.
          deadnix.glob = "*.nix";
          deadnix.check = "${lib.getExe pkgs.deadnix} --fail --exclude Cargo.nix {{files}}";

          clippy.check = "cargo clippy --all-targets --all-features -- -D warnings";
          lock-check.check = "cargo metadata --locked --format-version 1 > /dev/null";

          # A stale Cargo.nix is invisible to cargo and only breaks in the Nix build.
          # Like `just crate2nix-check`, regenerate and compare the hash: the file is
          # generated, never edited, so a change here is the fix being staged next.
          crate2nix-check.check = ''
            before=$(sha256sum Cargo.nix)
            ${crate2nix} generate > /dev/null
            after=$(sha256sum Cargo.nix)
            if [ "$before" != "$after" ]; then
              echo "Cargo.nix is out of step; run \`just crate2nix\` and stage it" >&2
              exit 1
            fi
          '';

          # The offline query cache, checked the way nix/_package.nix checks it: that
          # build sets SQLX_OFFLINE and has no database, so a stale .sqlx/ fails there.
          # This reproduces the failure for the price of an incremental cargo check.
          # Regenerate with `just sqlx-prepare`.
          sqlx-offline.check = "SQLX_OFFLINE=true cargo check --workspace --all-targets";
        };

        "commit-msg".steps.conventional.builtin = config.hk-nix.builtins.check_conventional_commit;
      };
    };
}
