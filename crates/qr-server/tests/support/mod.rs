//! The assembled router, and the few things every flow test needs to talk to it.
//!
//! Separate from the tests that use it so a flow test reads as the flow rather than as
//! twelve lines of request building.

#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use clap::Parser as _;
use qr_core::{
    BlobStore, CodeId, FsBlobStore, MintedCode, Secret, ids::DEFAULT_CODE_ID_LEN,
    ids::DEFAULT_SECRET_LEN, store::mint_codes, testing::TestDb,
};
use qr_server::cli::{Args, Command, ServeArgs};
use tower::ServiceExt as _;

pub fn serve_args() -> ServeArgs {
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

pub async fn router(db: &TestDb, dir: &tempfile::TempDir) -> Router {
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

/// Mints codes straight into the test's database, keeping their plaintext secrets.
pub async fn mint(db: &TestDb, count: usize) -> Vec<MintedCode> {
    let mut conn = db.pool().acquire().await.expect("connection");
    mint_codes(
        &mut conn,
        count,
        DEFAULT_CODE_ID_LEN,
        DEFAULT_SECRET_LEN,
        None,
        None,
        &serde_json::json!({}),
    )
    .await
    .expect("mint")
}

pub fn cookie(code: &CodeId, secret: &Secret) -> String {
    format!("qc_{code}={}", secret.expose())
}

pub async fn send(router: &Router, request: Request<Body>) -> Response {
    router.clone().oneshot(request).await.expect("response")
}

pub async fn get(router: &Router, uri: &str, cookie: Option<String>) -> Response {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(router, builder.body(Body::empty()).expect("request")).await
}

pub async fn claim(
    router: &Router,
    code: &CodeId,
    secret: &Secret,
    handler: &str,
    config: serde_json::Value,
) -> Response {
    let body = serde_json::json!({ "handler": handler, "config": config });
    send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{code}/api/v1/claim"))
            .header(header::COOKIE, cookie(code, secret))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request"),
    )
    .await
}

pub async fn text(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn location(response: &Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

pub fn set_cookie(response: &Response) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
pub fn assert_status(response: &Response, expected: StatusCode) {
    assert_eq!(
        response.status(),
        expected,
        "unexpected status; location was {}",
        location(response)
    );
}
