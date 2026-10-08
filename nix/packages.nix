{ ... }:
{
  perSystem =
    { pkgs, lib, ... }:
    let
      qr-server = pkgs.callPackage ./_package.nix { };
    in
    {
      packages = {
        default = qr-server;
        inherit qr-server;
      }
      # dockerTools only builds on Linux. CI runs there, so the push path has it; a
      # Darwin checkout still gets a devshell, a build and the tests.
      // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
        image = pkgs.callPackage ./_image.nix { inherit qr-server; };
      };

      checks = {
        inherit qr-server;

        # The #[ignore]d tests, against a Postgres this starts in the sandbox.
        integration = pkgs.callPackage ./_integration.nix { };

        # A green build says nothing about whether the binary starts. --check-config
        # parses and validates everything and exits before opening a socket or a
        # connection, so this catches a broken wrapper or a flag that no longer exists.
        qr-server-starts = pkgs.runCommand "qr-server-starts" { nativeBuildInputs = [ qr-server ]; } ''
          qr-server serve \
            --bind=0.0.0.0:8080 --health-bind=0.0.0.0:9090 \
            --base-url=https://qr.shine.town --blob-root=/data \
            --check-config

          # The sheet path forks typst, so this is also what proves the wrapper put
          # it on PATH and that the embedded templates and presets came along.
          qr-server sheet --calibration --preset relief-t7160 --out "$TMPDIR/cal.pdf"
          head -c4 "$TMPDIR/cal.pdf" | grep -q '%PDF'

          qr-server budget > /dev/null
          touch $out
        '';
      };

      apps.default = {
        type = "app";
        program = lib.getExe qr-server;
        meta.description = "Serve qr.shine.town, print a sheet, or size a QR";
      };
    };
}
