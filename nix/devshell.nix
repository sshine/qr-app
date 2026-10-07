{ inputs, ... }:
{
  imports = [ inputs.devshell.flakeModule ];
  perSystem =
    { config, pkgs, ... }:
    let
      rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
    in
    {
      devshells.default = {
        packages = [
          rust-toolchain
          config.treefmt.build.wrapper
          config.hk-nix.package
          pkgs.deadnix
          # buildRustPackage gets this from stdenv; a bare `cargo build` here needs a
          # linker named on PATH.
          pkgs.stdenv.cc
          pkgs.git
          pkgs.just

          # The same typst nix/_package.nix wraps the binary with, so a sheet rendered
          # by hand and a sheet rendered by the pod come out of one compiler.
          pkgs.typst

          # `just db-*`: initdb, pg_ctl, createdb, psql. The server rather than libpq,
          # because the local database is started from this shell.
          pkgs.postgresql

          # `just migrate` and `just sqlx-prepare`.
          pkgs.sqlx-cli

          # Pushing an image or a chart by hand, when CI is not the one doing it.
          pkgs.skopeo
          pkgs.kubernetes-helm
          pkgs.yq-go
          pkgs.jq
        ];

        env = [
          {
            name = "RUST_BACKTRACE";
            value = "1";
          }
          # Points at what `just db-up` starts. sqlx-cli, the query! macros and a bare
          # `cargo run` all read this one variable.
          {
            name = "DATABASE_URL";
            value = "postgres://qr:qr@127.0.0.1:5433/qr";
          }
          # Photo blobs. Production mounts a PVC here; locally it is a gitignored
          # directory, so `rm -rf` is the whole reset.
          {
            name = "QR_DATA_DIR";
            eval = "$PRJ_ROOT/.local/photos";
          }
          # What the QR codes encode. A sticker printed from a dev shell must not
          # resolve to production, so this differs from the deployed value.
          {
            name = "QR_PUBLIC_BASE_URL";
            value = "http://127.0.0.1:8080";
          }
        ];

        devshell.motd = "";
        devshell.startup.hk.text = config.hk-nix.shellHook;
      };

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
