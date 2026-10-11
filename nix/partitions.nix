# Headed Chromium is 574 MB on top of a devshell that is already large, and
# nothing but interactive debugging wants it: `playwright test --headed`, `--ui`
# mode and `codegen`. A partition keeps it out of the evaluation that every
# ordinary `nix develop` performs, so it costs nothing until asked for.
#
# Unlike the usual motivation for partitions, there are no extra inputs to scope
# out here; the browser comes from the nixpkgs this flake already pins. What
# this buys is evaluation scoping now, and somewhere to put a separate browser
# pin later without disturbing the default shell.
{ inputs, ... }:
{
  imports = [ inputs.flake-parts.flakeModules.partitions ];

  partitionedAttrs.devShells = "headed";
  partitions.headed.module = ./_headed.nix;
}
