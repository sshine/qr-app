# The tests that need a database and a typst, run inside the Nix sandbox.
#
# Those tests are #[ignore]d rather than feature-gated, so `--all-features` cannot switch
# them on where there is no server; this is the derivation that switches them on
# deliberately.
#
# One runner per crate, over the same crate2nix instance nix/_clippy.nix lints with. The
# compile is shared with it: `testCrateFlags`, `testInputs` and `testPreRun` are read by
# the runner alone, so asking for `--include-ignored` here rather than `--list` there
# costs another run of binaries that were already built, not another build of them.
#
# Postgres listens on loopback rather than a unix socket. The sandbox has no network but
# it does have a loopback interface, and each build gets a namespace of its own, so a
# fixed port cannot collide with another build.
{
  lib,
  callPackage,
  postgresql,
  typst,
  runCommand,
  ...
}:
let
  cargoNix = callPackage ./_cargo-nix.nix { } { lint = true; };

  port = "5433";

  # Which crate wants what, rather than handing every crate everything: a Postgres per
  # crate that has no use for one is six seconds of initdb for nothing.
  needsDatabase = [
    "qr-core"
    "qr-server"
  ];
  needsTypst = [
    "qr-sheet"
    "qr-server"
  ];

  wants = crate: list: builtins.elem crate list;

  # Guarded because crate2nix runs this once per test executable, and a crate with two of
  # them would otherwise initdb over a cluster that is already up. The loop runs in one
  # shell, so the flag survives between iterations.
  database = ''
    if [ -z "''${QR_PG_READY:-}" ]; then
      export PGDATA="$TMPDIR/pgdata"
      initdb --username postgres --auth trust --encoding UTF8 --no-locale >/dev/null
      pg_ctl --pgdata "$PGDATA" --log "$TMPDIR/pg.log" \
        --options "-p ${port} -c listen_addresses=127.0.0.1 -k $TMPDIR" \
        --wait start
      export QR_TEST_DATABASE_URL="postgres://postgres@127.0.0.1:${port}/postgres"
      QR_PG_READY=1
    fi
  '';

  tested = lib.mapAttrsToList (
    crate: member:
    member.build.override {
      features = cargoNix.allFeatures crate;
      runTests = true;
      # The whole reason this derivation exists, as against the suite the devshell runs.
      testCrateFlags = [ "--include-ignored" ];
      testInputs =
        lib.optional (wants crate needsDatabase) postgresql ++ lib.optional (wants crate needsTypst) typst;
      testPreRun =
        lib.optionalString (wants crate needsDatabase) database
        + lib.optionalString (wants crate needsTypst) ''
          export QR_TYPST_BIN="${lib.getExe typst}"
        '';
    }
  ) cargoNix.workspaceMembers;
in
runCommand "qr-integration"
  {
    # Naming them here is what makes them run; nothing is read back out.
    inherit tested;
    meta.description = "The qr tests that need a database and a typst";
  }
  ''
    for crate in $tested; do
      echo "tested $crate"
    done
    touch $out
  ''
