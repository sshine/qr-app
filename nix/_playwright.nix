# Playwright with a chosen browser set.
#
# nixpkgs' playwright-test bakes the full browser set into its wrapper, so it
# keeps a runtime reference to all 2.2 GB of Chromium, Firefox and WebKit no
# matter what PLAYWRIGHT_BROWSERS_PATH says at runtime. Retargeting it here takes
# the closure from 2345 MB to 634 MB for a suite that only ever drives Chromium.
#
# The agents talk to `playwright run-test-mcp-server`, which ships in this same
# package and honours the channel from playwright.config.ts. The separate
# playwright-mcp package would need headed Chromium (it resolves its browser as
# "chrome-for-testing" and will not fall back to the headless shell), so it is
# deliberately absent.
#
# Not a flake-parts module: import-tree skips the underscore prefix.
{ pkgs, withChromium }:
let
  browsers = pkgs.playwright-driver.browsers.override {
    inherit withChromium;
    withFirefox = false;
    withWebkit = false;
  };

  playwright-test = pkgs.playwright-test.overrideAttrs (old: {
    installPhase =
      builtins.replaceStrings [ "${pkgs.playwright-driver.browsers}" ] [ "${browsers}" ]
        old.installPhase;
  });
in
{
  inherit browsers playwright-test;
}
