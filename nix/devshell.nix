{ inputs, ... }:
{
  imports = [ inputs.devshell.flakeModule ];
  perSystem =
    {
      config,
      pkgs,
      ...
    }:
    {
      # Headless only: the chromium headless shell is what Playwright reaches for
      # in headless runs anyway. Headed Chromium is another 574 MB and lives in
      # the `headed` partition; see nix/partitions.nix.
      devshells.default.imports = [
        (import ./_devshell-common.nix {
          inherit config pkgs;
          withChromium = false;
        })
      ];

      # Helm and nothing else. The release workflow pushes the chart from here so an
      # empty-store runner does not fetch a Rust toolchain to run `helm push`.
      devshells.chart = {
        packages = [
          pkgs.kubernetes-helm
          pkgs.yq-go
          pkgs.just
          pkgs.git
        ];
        devshell.motd = "";
      };
    };
}
