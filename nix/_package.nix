# The build definition, shared by this flake's packages and by the overlay.
#
# Taking `pkgs` as arguments rather than closing over this flake's own is what lets the
# overlay build against a consumer's nixpkgs.
#
# Built through crate2nix rather than buildRustPackage: one derivation per crate instead
# of one for the whole workspace, so an unchanged dependency is a store hit rather than a
# recompile, and the lint check gets the same store paths rather than a second compile.
# Cargo.nix is generated and committed; `just crate2nix` regenerates it and
# `just crate2nix-check` is what notices when it has fallen behind the manifests.
#
# The crate2nix instance itself lives in _cargo-nix.nix, shared with that check.
{
  lib,
  stdenv,
  callPackage,
  makeBinaryWrapper,
  typst,
  ...
}:
let
  cargoNix = callPackage ./_cargo-nix.nix { } { };
  unwrapped = cargoNix.workspaceMembers.qr-server.build;
in
stdenv.mkDerivation {
  pname = "qr-server";
  # Read rather than repeated: a release bumps one place and this cannot fall behind it.
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  dontUnpack = true;
  nativeBuildInputs = [ makeBinaryWrapper ];

  # --prefix, not --suffix: the flake's typst must win over whatever is ambient, or a
  # sheet rendered on a laptop and one rendered in the pod come out of different
  # compilers. This is also what carries typst into the image's closure, so
  # nix/_image.nix does not list it.
  #
  # makeBinaryWrapper rather than makeWrapper, which keeps bash out of the image.
  #
  # The cache path is set because typst writes there only if a template ever imports a
  # @preview package, and /tmp is the pod's one writable directory.
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    makeBinaryWrapper ${unwrapped}/bin/qr-server $out/bin/qr-server \
      --prefix PATH : ${lib.makeBinPath [ typst ]} \
      --set-default TYPST_PACKAGE_CACHE_PATH /tmp/typst-cache
    runHook postInstall
  '';

  meta = {
    description = "Pre-printed QR stickers, and what they open";
    homepage = "https://qr.shine.town";
    mainProgram = "qr-server";
    license = with lib.licenses; [
      mit
      asl20
    ];
    platforms = lib.platforms.unix;
  };
}
