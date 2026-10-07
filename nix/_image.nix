# The OCI image, built from the same derivation `nix flake check` verifies, so what ships
# is what was tested.
#
# streamLayeredImage rather than buildLayeredImage: it produces a script that writes the
# tarball to stdout, so CI never materialises a few hundred megabytes in the store. Pipe
# it into `skopeo copy docker-archive:/dev/stdin` or `docker load`. A build-time choice
# only; what comes out is an ordinary image.
#
# No Entrypoint and no Cmd. The manifest names the binary and the subcommand on both the
# init container and the server, which is what lets one image migrate and serve without
# an entrypoint script deciding between them.
{
  lib,
  dockerTools,
  cacert,
  runCommand,
  qr-server,
  name ? "git.shine.town/infra/qr",
  tag ? "latest",
}:
dockerTools.streamLayeredImage {
  inherit name tag;

  # Epoch, so the same inputs give the same digest.
  created = "1970-01-01T00:00:00Z";

  # typst is deliberately absent: the wrapper in nix/_package.nix refers to it by store
  # path, so it arrives in the closure anyway, and listing it here would put a second
  # copy on /bin that nothing calls.
  contents = [
    qr-server
    cacert
    (runCommand "qr-image-etc" { } ''
      mkdir -p $out/etc $out/tmp
      chmod 1777 $out/tmp

      # nsswitch.conf is not cosmetic: without it glibc does not consult DNS at all, and
      # the database's cluster-local hostname never resolves.
      echo 'hosts: files dns' > $out/etc/nsswitch.conf

      # 65534, matching the securityContext the chart renders. runAsNonRoot wants a
      # passwd entry to exist for the uid it is given.
      echo 'nobody:x:65534:65534:nobody:/nonexistent:/sbin/nologin' > $out/etc/passwd
      echo 'nogroup:x:65534:' > $out/etc/group
    '')
  ];

  config = {
    User = "65534:65534";
    Env = [
      "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt"
      "TYPST_PACKAGE_CACHE_PATH=/tmp/typst-cache"
    ];
    ExposedPorts = {
      "8080/tcp" = { };
      "9090/tcp" = { };
    };
    Labels = {
      "org.opencontainers.image.source" = "https://git.shine.town/sshine/qr";
      "org.opencontainers.image.licenses" = "MIT OR Apache-2.0";
      "org.opencontainers.image.description" = "Pre-printed QR stickers, and what they open";
    };
  };

  meta.platforms = lib.platforms.linux;
}
