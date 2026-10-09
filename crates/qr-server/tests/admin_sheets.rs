//! `POST /admin/sheets`, against a real database and a real typst.
//!
//! Driven through the assembled router rather than the handler, so the admin guard, the
//! body limit and the response headers are the ones a deployment serves. Ignored for the
//! same reason the other suites are: `nix flake check` and `just test-db` supply both.

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
use serde_json::{Value, json};
use tower::ServiceExt as _;

const TOKEN: &str = "a-static-token";

/// A router over a scratch database, and the database itself to count rows in.
struct Harness {
    db: TestDb,
    router: Router,
    // The blob root and the token file live here; dropping it takes both away.
    _dir: tempfile::TempDir,
}

/// Parses the flags rather than building the struct, so a flag renamed out from under
/// the chart is caught here too.
fn serve_args(extra: &[String]) -> ServeArgs {
    let argv = ["qr-server".to_owned(), "serve".to_owned()]
        .into_iter()
        .chain(extra.iter().cloned());
    match Args::try_parse_from(argv).expect("arguments").command {
        Command::Serve(serve) => *serve,
        _ => panic!("expected serve"),
    }
}

async fn harness() -> Harness {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");

    let token_file = dir.path().join("admin-token");
    // With a trailing newline, which is what a mounted Kubernetes secret looks like.
    std::fs::write(&token_file, format!("{TOKEN}\n")).expect("token file");

    let typst = std::env::var("QR_TYPST_BIN").unwrap_or_else(|_| "typst".to_owned());
    let args = serve_args(&[
        format!("--admin-token-file={}", token_file.display()),
        format!("--typst-bin={typst}"),
        "--base-url=https://qr.shine.town".to_owned(),
    ]);

    let blobs: Arc<dyn BlobStore> = Arc::new(
        FsBlobStore::open(dir.path().join("blobs"))
            .await
            .expect("blob store"),
    );
    let registry = qr_server::app::registry().expect("registry");
    let state = qr_server::app::state(&args, db.store(), blobs, registry)
        .await
        .expect("state");
    let admin = qr_server::admin::state(&args, state.clone()).expect("admin state");

    Harness {
        db,
        router: qr_server::app::router(state, admin),
        _dir: dir,
    }
}

impl Harness {
    /// Sends a request and returns the status, the headers and the whole body.
    async fn post(
        &self,
        body: &Value,
        token: Option<&str>,
    ) -> (StatusCode, header::HeaderMap, Vec<u8>) {
        let mut request = Request::builder()
            .method("POST")
            .uri("/admin/sheets")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = request.body(Body::from(body.to_string())).expect("request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        (status, headers, bytes.to_vec())
    }

    async fn codes(&self) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM codes")
            .fetch_one(self.db.pool())
            .await
            .expect("count codes")
    }

    async fn sheets(&self) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sheets")
            .fetch_one(self.db.pool())
            .await
            .expect("count sheets")
    }
}

fn header(headers: &header::HeaderMap, name: header::HeaderName) -> String {
    headers
        .get(&name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
#[ignore = "needs a database and typst"]
async fn a_sheet_is_minted_and_answered_with_the_pdf() {
    let h = harness().await;
    let (status, headers, body) = h
        .post(&json!({ "preset": "plain", "count": 3 }), Some(TOKEN))
        .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(header(&headers, header::CONTENT_TYPE), "application/pdf");
    assert!(body.starts_with(b"%PDF"), "not a PDF: {:?}", &body[..8]);

    // The sheet id rides on the filename, so a script has it without a header of our own.
    let sheet = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM sheets")
        .fetch_one(h.db.pool())
        .await
        .expect("the sheet row");
    assert_eq!(
        header(&headers, header::CONTENT_DISPOSITION),
        format!("attachment; filename=\"sheet-{sheet}.pdf\"")
    );

    // The response is the only copy of these secrets, and no cache should hold it.
    assert_eq!(header(&headers, header::CACHE_CONTROL), "no-store");

    assert_eq!(h.codes().await, 3);
}

#[tokio::test]
#[ignore = "needs a database and typst"]
async fn an_omitted_count_fills_one_sheet_of_the_stock() {
    let h = harness().await;
    let (status, _, _) = h.post(&json!({ "preset": "plain" }), Some(TOKEN)).await;

    assert_eq!(status, StatusCode::CREATED);
    // `plain` is 4 by 3.
    assert_eq!(h.codes().await, 12);
}

#[tokio::test]
#[ignore = "needs a database and typst"]
async fn a_handler_named_in_the_request_is_what_every_code_is_minted_for() {
    let h = harness().await;
    let (status, _, _) = h
        .post(
            &json!({ "preset": "plain", "count": 2, "handler": "photo-album" }),
            Some(TOKEN),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);
    let handlers = sqlx::query_scalar::<_, Option<String>>("SELECT intended_handler FROM codes")
        .fetch_all(h.db.pool())
        .await
        .expect("handlers");
    assert_eq!(handlers, vec![Some("photo-album".to_owned()); 2]);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_request_without_a_token_mints_nothing() {
    let h = harness().await;
    let (status, _, _) = h
        .post(&json!({ "preset": "plain", "count": 3 }), None)
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(h.sheets().await, 0);
    assert_eq!(h.codes().await, 0);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_request_with_the_wrong_token_mints_nothing() {
    let h = harness().await;
    let (status, _, _) = h
        .post(&json!({ "preset": "plain" }), Some("not-the-token"))
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(h.codes().await, 0);
}

/// Each of these is refused before the transaction opens, so a bad request costs no
/// codes. That is the property worth holding: a code id, once minted, is spent.
#[tokio::test]
#[ignore = "needs a database"]
async fn nothing_is_minted_for_a_request_that_cannot_be_served() {
    let h = harness().await;

    for body in [
        json!({ "preset": "no-such-stock" }),
        json!({ "preset": "plain", "handler": "no-such-handler" }),
        json!({ "preset": "plain", "count": 0 }),
        json!({ "preset": "plain", "count": 1001 }),
        json!({ "preset": "plain", "ec": "Z" }),
    ] {
        let (status, _, body_out) = h.post(&body, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} gave {status}");
        assert!(
            !body_out.is_empty(),
            "{body} was refused without saying why"
        );
    }

    assert_eq!(h.sheets().await, 0);
    assert_eq!(h.codes().await, 0);
}

/// A misspelled field would otherwise be dropped in silence, and the mistake would be
/// found on the boxes rather than in the response.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_field_this_route_does_not_know_is_refused() {
    let h = harness().await;
    let (status, _, _) = h
        .post(
            &json!({ "preset": "plain", "handlr": "photo-album" }),
            Some(TOKEN),
        )
        .await;

    // axum's own rejection for JSON that parses but does not fit the shape.
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(h.codes().await, 0);
}

/// The route is the only one under /admin, and nothing else there should answer.
#[tokio::test]
#[ignore = "needs a database"]
async fn the_admin_subtree_has_nothing_else_in_it() {
    let h = harness().await;
    let response = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/sheets")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
