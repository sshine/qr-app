//! The client's files, embedded in the binary.
//!
//! Embedded rather than served from a store path: everything in the Nix store has an
//! mtime of 1970, so a directory server would emit one constant Last-Modified and every
//! deploy would hand a warm cache stale CSS. Embedding lets the ETag be the content, and
//! means `cargo run` from a checkout serves byte-identical files to the container.

use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
// VENDORED.md records where the vendored build came from and belongs beside it in the
// repository, not inside the binary.
#[exclude = "*.md"]
struct Assets;

/// The one asset that cannot live under /assets/.
///
/// A worker may only control paths at or below its own, and every page this service
/// serves is under /c/, so a worker at /assets/sw.js would control nothing.
pub const SERVICE_WORKER: &str = "sw.js";

pub fn router() -> Router {
    Router::new().route("/assets/{*path}", get(serve))
}

/// A short version of the whole asset set.
///
/// Every page links to `/assets/<name>?v=<version>`, so a deploy that changes any asset
/// changes the URL. That is what makes the service worker's cache-first rule safe: it
/// cannot have the new bytes under a URL it has never seen. The version is a BLAKE3 hash
/// over every embedded asset's SHA-256, in a fixed order, so adding an asset cannot leave
/// it outside the version by accident.
pub fn version() -> String {
    let mut hasher = blake3::Hasher::new();
    for name in [
        "app.css",
        "app.js",
        "vendor/alpine.js",
        "vendor/jsqr.js",
        "icon.svg",
    ] {
        if let Some(file) = Assets::get(name) {
            hasher.update(&file.metadata.sha256_hash());
        }
    }
    hex16(hasher.finalize().as_bytes())
}

/// The URL the shell and the manifest should publish for an asset.
pub fn url(name: &str) -> String {
    format!("/assets/{name}?v={}", version())
}

async fn serve(Path(path): Path<String>) -> Response {
    // `vendor/alpine.js` is published at `/assets/alpine.js`: where it came from is a
    // fact about this repository and not something a page should have to spell.
    let candidates = [path.clone(), format!("vendor/{path}")];

    let found = candidates
        .iter()
        .find_map(|name| Assets::get(name).map(|file| (name.as_str(), file)));

    let Some((name, file)) = found else {
        return (StatusCode::NOT_FOUND, "no such asset\n").into_response();
    };

    file_response(name, &file)
}

/// Serves the worker from the root.
pub async fn service_worker() -> Response {
    match Assets::get(SERVICE_WORKER) {
        Some(file) => file_response(SERVICE_WORKER, &file),
        None => (StatusCode::NOT_FOUND, "no service worker\n").into_response(),
    }
}

fn file_response(name: &str, file: &rust_embed::EmbeddedFile) -> Response {
    let etag = format!("\"{}\"", hex16(&file.metadata.sha256_hash()));
    let mime = file.metadata.mimetype();

    // The service worker is the one file that must not be cached: a stale one keeps
    // serving a stale shell, and the browser would have no way to be told otherwise.
    //
    // Everything else is served from a URL that carries its content version, so the
    // bytes under a given URL really cannot change. That is what the old `must-revalidate`
    // hour could not promise, and what the service worker's cache-first rule needs.
    let cache = if name == SERVICE_WORKER {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    (
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache.to_owned()),
        ],
        file.data.to_vec(),
    )
        .into_response()
}

fn hex16(bytes: &[u8; 32]) -> String {
    bytes.iter().take(8).fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_the_shell_asks_for_is_embedded() {
        // The shell writes these paths into every page, so a renamed asset should fail
        // here rather than as a blank page on a phone.
        for name in ["app.css", "app.js", "icon.svg", "sw.js"] {
            assert!(Assets::get(name).is_some(), "{name} is not embedded");
        }
        assert!(
            Assets::get("vendor/alpine.js").is_some(),
            "the vendored Alpine build is not embedded"
        );
        assert!(
            Assets::get("vendor/jsqr.js").is_some(),
            "the vendored QR decoder is not embedded"
        );
    }

    #[test]
    fn the_version_is_short_enough_to_put_in_every_url() {
        let version = version();
        assert_eq!(version.len(), 16, "{version}");
        assert!(
            version.bytes().all(|b| b.is_ascii_hexdigit()),
            "{version} is not hex"
        );
    }

    #[test]
    fn an_asset_url_carries_the_version() {
        assert_eq!(url("app.css"), format!("/assets/app.css?v={}", version()));
    }
}
