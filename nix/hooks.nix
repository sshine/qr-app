{ inputs, ... }:
{
  imports = [ inputs.hk-nix.flakeModules.default ];
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      # By absolute store path: the `nix flake check` hk-check sandbox runs hooks
      # without the devshell PATH, so a relative `treefmt` is not found.
      treefmt = lib.getExe config.treefmt.build.wrapper;
      betterleaks = lib.getExe pkgs.betterleaks;
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
          deadnix.glob = "*.nix";
          deadnix.check = "${lib.getExe pkgs.deadnix} --fail {{files}}";

          clippy.check = "cargo clippy --all-targets --all-features -- -D warnings";
          lock-check.check = "cargo metadata --locked --format-version 1 > /dev/null";

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
