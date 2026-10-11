# The `headed` partition's module: `nix develop .#headed`. Not imported by
# import-tree: the underscore keeps it out of the base evaluation, which is the
# point. Declared in nix/partitions.nix.
{
  perSystem =
    {
      config,
      pkgs,
      ...
    }:
    {
      devshells.headed = {
        imports = [
          (import ./_devshell-common.nix {
            inherit config pkgs;
            withChromium = true;
          })
        ];

        # The suite runs headed or headless from this one shell; the config reads
        # this to decide whether to pin the headless shell as its channel.
        env = [
          {
            name = "PW_HEADED";
            value = "1";
          }
        ];
      };
    };
}
