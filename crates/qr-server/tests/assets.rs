//! The assets a page links to, and why a returning phone cannot keep the old ones.
//!
//! The service worker is cache-first for `/assets/*`. That is only correct if the URL
//! changes with the bytes, so these assert the contract between the shell and the
//! server: one version, in every URL, and served as immutable.

#![allow(clippy::expect_used)]

mod support;

use axum::http::{StatusCode, header};
use qr_core::testing::TestDb;

#[tokio::test]
#[ignore = "needs a database"]
async fn a_page_publishes_versioned_assets_and_the_version_is_what_it_serves() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;

    let version = qr_server::assets::version();
    assert_eq!(version.len(), 16, "{version}");

    let response = support::get(&router, "/", None).await;
    support::assert_status(&response, StatusCode::OK);
    let page = support::text(response).await;

    for name in ["app.css", "app.js", "alpine.js", "jsqr.js"] {
        let url = format!("/assets/{name}?v={version}");
        assert!(
            page.contains(&url),
            "the page does not link to {url}:\n{page}"
        );
    }

    let css_url = format!("/assets/app.css?v={version}");
    let response = support::get(&router, &css_url, None).await;
    support::assert_status(&response, StatusCode::OK);
    let cache = response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        cache.contains("immutable"),
        "a versioned asset is not immutable: {cache}"
    );
    let css = support::text(response).await;
    assert!(
        css.contains(".card-body"),
        "the served stylesheet is not the one the page was built with"
    );

    // The camera decoder is part of the versioned shell too, so a deploy cannot pair
    // new markup with a decoder a warm cache has never seen.
    let response = support::get(&router, &format!("/assets/jsqr.js?v={version}"), None).await;
    support::assert_status(&response, StatusCode::OK);
    let decoder = support::text(response).await;
    assert!(
        decoder.contains("jsQR"),
        "the served decoder is not the vendored jsQR build"
    );

    let response = support::get(&router, "/manifest.webmanifest", None).await;
    support::assert_status(&response, StatusCode::OK);
    let manifest = support::text(response).await;
    assert!(
        manifest.contains(&format!("/assets/icon.svg?v={version}")),
        "the manifest icon is unversioned:\n{manifest}"
    );
}
