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
    let cache = if name == SERVICE_WORKER {
        "no-cache"
    } else {
        "public, max-age=3600, must-revalidate"
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
    }
}
