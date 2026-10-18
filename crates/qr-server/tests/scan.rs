//! What a scanned sticker reaches.
//!
//! A QR carries the URL uppercased, so the address a camera hands the browser is not the
//! one the router registers. Nothing exercised that before, and the mismatch reached a
//! deployment: every scan answered "Nothing here". These drive the assembled router, so
//! the spelling under test is the one a phone sends.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use clap::Parser as _;
use qr_core::{BlobStore, FsBlobStore, testing::TestDb};
use qr_server::cli::{Args, Command, ServeArgs};
use tower::ServiceExt as _;

/// A code id in the shape a sheet prints, and a sticker encodes.
const CODE: &str = "BFCADPV0QA";

fn serve_args() -> ServeArgs {
    let argv = [
        "qr-server".to_owned(),
        "serve".to_owned(),
        "--base-url=https://qr.shine.town".to_owned(),
    ];
    match Args::try_parse_from(argv).expect("arguments").command {
        Command::Serve(serve) => *serve,
        _ => panic!("expected serve"),
    }
}

async fn router(db: &TestDb, dir: &tempfile::TempDir) -> Router {
    let args = serve_args();
    let blobs: Arc<dyn BlobStore> = Arc::new(
        FsBlobStore::open(dir.path().join("blobs"))
            .await
            .expect("blob store"),
    );
    let registry = qr_server::app::registry().expect("registry");
    let state = qr_server::app::state(&args, db.store(), blobs, registry)
        .await
        .expect("state");
    let sheets = qr_server::sheet::routes::state(&args, state.clone()).expect("sheet state");
    qr_server::app::router(state, sheets)
}

async fn get(router: &Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, location)
}

async fn body(router: &Router, uri: &str) -> String {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK, "{uri} did not answer");
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The front page has to offer the generator, and offering it must not spend a sheet.
///
/// `/api/v1/sheets` mints on GET, so a link to it from the landing page is spent by the
/// first crawler that follows it and by every browser that prefetches. The page in
/// between carries a form instead, which neither of them submits.
#[tokio::test]
#[ignore = "needs a database"]
async fn the_front_page_reaches_the_generator_without_minting() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = router(&db, &dir).await;

    let landing = body(&router, "/").await;
    assert!(
        landing.contains(r#"href="/sheet""#),
        "the front page does not reach the generator:\n{landing}"
    );
    assert!(
        !landing.contains("/api/v1/sheets"),
        "the front page links straight at the minting route:\n{landing}"
    );

    let form = body(&router, "/sheet").await;
    assert!(
        form.contains(r#"action="/api/v1/sheets""#),
        "the generator page has no form to submit:\n{form}"
    );
    assert!(
        form.contains(qr_sheet::DEFAULT_PRESET),
        "the form names no stock, so the request would be refused:\n{form}"
    );
}

/// The whole point: what a camera sends has to reach the code, not the 404 page.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_scanned_sticker_reaches_its_code() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = router(&db, &dir).await;

    let (status, location) = get(&router, &format!("/C/{CODE}")).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT, "a scan 404'd");
    assert_eq!(location, format!("/c/{CODE}"));

    // And the destination answers, rather than redirecting onwards.
    let (status, _) = get(&router, &location).await;
    assert_eq!(status, StatusCode::OK);
}

/// Typed in by hand, in whatever case the keyboard produced.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_shouted_url_is_normalised_to_the_canonical_spelling() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = router(&db, &dir).await;

    let (status, location) = get(&router, &format!("/C/{}", CODE.to_lowercase())).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location, format!("/c/{CODE}"), "not the canonical spelling");
}

/// Not a code at all. The typed form is the way back in, so this is a page and not a
/// redirect to one that cannot exist.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_shouted_url_that_is_not_a_code_still_answers() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = router(&db, &dir).await;

    // Too short to be an id. Note that "NOT-A-CODE" is not a good example: separators
    // are stripped and Crockford reads O as 0, so it parses as one.
    let (status, _) = get(&router, "/C/X").await;
    assert_eq!(status, StatusCode::OK);
}

/// The bootstrap page has to hand itself to Alpine, or the exchange never runs.
///
/// Alpine only instantiates a component where the markup carries `x-data`. The page
/// defines `Alpine.data("bootstrap", ...)`, whose `init` reads the fragment and redeems
/// it, and a visitor whose markup lacks the attribute is left looking at the typed form
/// with their secret sitting unused in the address bar.
#[tokio::test]
#[ignore = "needs a database"]
async fn the_bootstrap_page_starts_the_exchange() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = router(&db, &dir).await;

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/c/{CODE}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&body);

    assert!(
        html.contains("class=\"bootstrap\""),
        "not the bootstrap page"
    );
    assert!(
        html.contains("x-data=\"bootstrap\""),
        "the bootstrap page does not start its Alpine component, so the fragment is \
         never exchanged:\n{html}"
    );
}
