{ ... }:
{
  flake.overlays.default = final: _prev: {
    qr-server = final.callPackage ./_package.nix { };
  };
}
