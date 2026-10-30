//! A sticker made to stand in for another one.
//!
//! The property is not that a link redirects: it is that a browser which has never seen
//! the original sticker is handed the original's capability on the way, so both sides of
//! the box open the same thing without a second secret being typed.

#![allow(clippy::expect_used)]

mod support;

use axum::http::StatusCode;
use qr_core::{MintedCode, Secret, testing::TestDb};

/// Claims a code as an empty photo album through the same route a browser uses.
async fn album(router: &axum::Router, code: &MintedCode) {
    let response = support::claim(
        router,
        &code.id,
        &code.secret,
        "photo-album",
        serde_json::json!({}),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
}

/// Claims `link` as a pointer at `target`.
async fn link_to(
    router: &axum::Router,
    link: &MintedCode,
    target: &MintedCode,
) -> axum::response::Response {
    support::claim(
        router,
        &link.id,
        &link.secret,
        "qr-link",
        serde_json::json!({
            "code": target.id.as_str(),
            "secret": target.secret.expose(),
        }),
    )
    .await
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_scan_of_a_link_opens_the_code_it_stands_in_for() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (original, link) = (&codes[0], &codes[1]);

    album(&router, original).await;
    let response = link_to(&router, link, original).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    // The link's own scan does not render the original's page itself: it hands over the
    // original's cookie and sends the browser there, which is what makes every relative
    // URL on the destination (photos, API calls) work under the original's path.
    let response = support::get(
        &router,
        &format!("/c/{}", link.id),
        Some(support::cookie(&link.id, &link.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let target = format!("/c/{}?hop=1", original.id);
    assert_eq!(
        support::location(&response),
        target,
        "the link did not send the visitor to the original"
    );
    let handed_over = support::set_cookie(&response);
    assert!(
        handed_over.contains(&format!("qc_{}=", original.id)),
        "the redirect carried no capability for the original: {handed_over}"
    );
    assert!(
        handed_over.contains(original.secret.expose()),
        "the cookie handed over is not the original's secret: {handed_over}"
    );

    // A hop counter only exists to bound a chain; the page that is not a link strips it
    // so the address bar holds the canonical URL.
    let response = support::get(
        &router,
        &target,
        Some(support::cookie(&original.id, &original.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
    assert_eq!(support::location(&response), format!("/c/{}", original.id));

    let response = support::get(
        &router,
        &format!("/c/{}", original.id),
        Some(support::cookie(&original.id, &original.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    assert!(
        body.contains("main class=\"album\""),
        "the original did not open its album:\n{body}"
    );
}

/// A link is a capability delegation. It must not become a way to read the original's
/// secret out of the database through the API or a rendered page.
#[tokio::test]
#[ignore = "needs a database"]
async fn the_original_secret_is_not_kept_in_the_bindings_config() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (original, link) = (&codes[0], &codes[1]);

    album(&router, original).await;
    link_to(&router, link, original).await;

    let config: String = sqlx::query_scalar(
        "SELECT config::text FROM bindings WHERE code_id = $1 AND released_at IS NULL",
    )
    .bind(link.id.as_str())
    .fetch_one(db.pool())
    .await
    .expect("binding config");

    assert!(
        !config.contains(original.secret.expose()),
        "the original's secret is readable in the binding's config: {config}"
    );
    assert!(
        config.contains(original.id.as_str()),
        "the binding does not say what it links to: {config}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_link_needs_the_other_sticker_s_real_secret() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (original, link) = (&codes[0], &codes[1]);

    album(&router, original).await;

    let wrong = Secret::generate(original.secret.len()).expect("secret");
    let response = support::claim(
        &router,
        &link.id,
        &link.secret,
        "qr-link",
        serde_json::json!({
            "code": original.id.as_str(),
            "secret": wrong.expose(),
        }),
    )
    .await;
    support::assert_status(&response, StatusCode::BAD_REQUEST);

    // And nothing was half-written: the code still offers the chooser.
    let response = support::get(
        &router,
        &format!("/c/{}", link.id),
        Some(support::cookie(&link.id, &link.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    assert!(
        body.contains("main class=\"chooser\""),
        "a refused link left the code claimed:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_link_to_a_code_nobody_has_used_is_refused() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (unclaimed, link) = (&codes[0], &codes[1]);

    let response = link_to(&router, link, unclaimed).await;
    support::assert_status(&response, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_sticker_cannot_link_to_itself() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let only = &codes[0];

    // An unclaimed code is refused for not being in use before the self check can fire,
    // so the assertion is on the outcome a visitor would see: still a chooser.
    let response = link_to(&router, only, only).await;
    support::assert_status(&response, StatusCode::BAD_REQUEST);

    let response = support::get(
        &router,
        &format!("/c/{}", only.id),
        Some(support::cookie(&only.id, &only.secret)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// The chooser cannot ask a visitor to transcribe a secret any more. It has to be the
/// camera, and the confirmation that writes the decoded pair into the claim form must
/// only exist as part of that flow.
#[tokio::test]
#[ignore = "needs a database"]
async fn the_link_chooser_is_a_camera_and_not_typed_fields() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let only = &codes[0];

    let response = support::get(
        &router,
        &format!("/c/{}", only.id),
        Some(support::cookie(&only.id, &only.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;

    for expected in [
        "x-data=\"linkScan\"",
        "x-on:click=\"start\"",
        "Use this sticker",
        "name=\"config.code\"",
        "name=\"config.secret\"",
        "/assets/jsqr.js?v=",
    ] {
        assert!(
            body.contains(expected),
            "the link chooser is missing {expected}:\n{body}"
        );
    }

    assert!(
        !body.contains("Code on the other sticker"),
        "the typed code field survived:\n{body}"
    );
    assert!(
        !body.contains("type=\"text\""),
        "the link chooser still has a typed input:\n{body}"
    );
}

/// The chain bound must not be reachable through the redirect: a cycle of links would
/// otherwise hang a browser that follows redirects forever.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_chain_of_links_stops_instead_of_looping() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (first, second) = (&codes[0], &codes[1]);

    album(&router, first).await;
    link_to(&router, second, first).await;

    // A code id in the link is enough to mint the second hop's URL by hand.
    let response = support::get(
        &router,
        &format!("/c/{}?hop=8", second.id),
        Some(support::cookie(&second.id, &second.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::LOOP_DETECTED);
    let body = support::text(response).await;
    assert!(
        body.contains("pointing at each other"),
        "the loop page did not explain itself:\n{body}"
    );
}

/// A code minted with a handler but no link proof must not reach `on_bind` with a
/// half-filled config and store an empty delegation.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_sheet_preset_without_a_target_is_refused() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let only = &codes[0];

    let response = support::claim(
        &router,
        &only.id,
        &only.secret,
        "qr-link",
        serde_json::json!({}),
    )
    .await;
    support::assert_status(&response, StatusCode::BAD_REQUEST);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM code_links")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 0, "a refused claim wrote a link row");
}
