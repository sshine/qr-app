//! Releasing a code, and getting it back.
//!
//! Release is a soft delete: the binding row is marked, not removed, and the handler's
//! own rows (photos, destinations, links) never move. Recovery is therefore one UPDATE,
//! and the property worth stating is that the contents are still there afterwards.

#![allow(clippy::expect_used)]

mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use qr_core::{MintedCode, store, testing::TestDb};
use uuid::Uuid;

async fn release(router: &axum::Router, code: &MintedCode) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/release", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .body(Body::empty())
            .expect("request"),
    )
    .await
}

async fn recover(
    router: &axum::Router,
    code: &MintedCode,
    binding: Uuid,
) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/recover", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("binding={binding}")))
            .expect("request"),
    )
    .await
}

async fn page(router: &axum::Router, code: &MintedCode) -> String {
    let response = support::get(
        router,
        &format!("/c/{}", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    support::text(response).await
}

/// A photo row with the three blobs it points at, without going through an upload.
///
/// The page only needs the row to render a tile; the bytes are the pipeline's business.
async fn add_photo(db: &TestDb, binding: Uuid) -> Uuid {
    let keys = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
    for key in &keys {
        sqlx::query("INSERT INTO blobs (key, byte_size, media_type) VALUES ($1, 1, 'image/jpeg')")
            .bind(key)
            .execute(db.pool())
            .await
            .expect("blob");
    }

    let id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO photo_items
            (id, binding_id, original_key, view_key, thumb_key, media_type,
             width, height, byte_size)
        VALUES ($1, $2, $3, $4, $5, 'image/jpeg', 1, 1, 1)
        "#,
    )
    .bind(id)
    .bind(binding)
    .bind(&keys[0])
    .bind(&keys[1])
    .bind(&keys[2])
    .execute(db.pool())
    .await
    .expect("photo");
    id
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_released_album_comes_back_with_its_photos() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];

    let response = support::claim(
        &router,
        &code.id,
        &code.secret,
        "photo-album",
        serde_json::json!({}),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let binding = store::active_binding(db.pool(), &code.id)
        .await
        .expect("binding lookup")
        .expect("binding");
    let photo = add_photo(&db, binding.id).await;

    let response = release(&router, code).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
    assert!(
        store::active_binding(db.pool(), &code.id)
            .await
            .expect("lookup")
            .is_none(),
        "release left the binding active"
    );

    // The chooser offers the history back, with what it held.
    let chooser = page(&router, code).await;
    assert!(
        chooser.contains("Recover QR code"),
        "a released code did not offer recovery:\n{chooser}"
    );
    assert!(
        chooser.contains("1 photo"),
        "the released album did not say what it held:\n{chooser}"
    );

    let response = recover(&router, code, binding.id).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let album = page(&router, code).await;
    assert!(
        album.contains("main class=\"album\""),
        "recovery did not restore the album:\n{album}"
    );
    assert!(
        album.contains(&format!("/photos/{photo}/view")),
        "the photo did not come back with the album:\n{album}"
    );

    // And the row is the same one, still not deleted.
    let deleted: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM photo_items WHERE id = $1")
            .bind(photo)
            .fetch_one(db.pool())
            .await
            .expect("photo row");
    assert!(deleted.is_none(), "the photo was deleted, not kept");
}

/// Releasing the second sticker on a box must not release the first one.
#[tokio::test]
#[ignore = "needs a database"]
async fn releasing_a_link_leaves_the_original_alone() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (original, link) = (&codes[0], &codes[1]);

    support::claim(
        &router,
        &original.id,
        &original.secret,
        "photo-album",
        serde_json::json!({}),
    )
    .await;
    support::claim(
        &router,
        &link.id,
        &link.secret,
        "qr-link",
        serde_json::json!({
            "code": original.id.as_str(),
            "secret": original.secret.expose(),
        }),
    )
    .await;

    let response = release(&router, link).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let original_binding = store::active_binding(db.pool(), &original.id)
        .await
        .expect("lookup")
        .expect("the original was released with the link");
    assert_eq!(original_binding.handler_id, "photo-album");

    let body = page(&router, original).await;
    assert!(
        body.contains("main class=\"album\""),
        "the original stopped opening:\n{body}"
    );

    let chooser = page(&router, link).await;
    assert!(
        chooser.contains("Recover QR code"),
        "the link did not offer its own recovery:\n{chooser}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_code_claimed_again_cannot_be_recovered_over() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];

    support::claim(
        &router,
        &code.id,
        &code.secret,
        "photo-album",
        serde_json::json!({}),
    )
    .await;
    let old = store::active_binding(db.pool(), &code.id)
        .await
        .expect("lookup")
        .expect("binding");

    release(&router, code).await;

    // A different use claims the code before anyone recovers the old one.
    let response = support::claim(
        &router,
        &code.id,
        &code.secret,
        "forward",
        serde_json::json!({ "url": "https://example.com/new" }),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let response = recover(&router, code, old.id).await;
    support::assert_status(&response, StatusCode::CONFLICT);

    let current = store::active_binding(db.pool(), &code.id)
        .await
        .expect("lookup")
        .expect("binding");
    assert_eq!(current.handler_id, "forward", "recovery replaced the claim");
}

/// A forward code does not redirect on the first request any more: it shows where it is
/// going, with the controls every issued code carries, and leaves after the countdown.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_forward_code_shows_its_destination_before_leaving() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];

    let destination = "https://example.com/a-poster";
    let response = support::claim(
        &router,
        &code.id,
        &code.secret,
        "forward",
        serde_json::json!({ "url": destination }),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let response = support::get(
        &router,
        &format!("/c/{}", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;

    for expected in [destination, "Continue now", "Stay here", "code-controls"] {
        assert!(
            body.contains(expected),
            "the interstitial is missing {expected}:\n{body}"
        );
    }

    // The editor is still reachable after the code started redirecting.
    let response = support::get(
        &router,
        &format!("/c/{}/manage", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    assert!(
        body.contains("Save") && body.contains(destination),
        "the manage page lost the destination editor:\n{body}"
    );
}

/// A visitor has to be able to release a code whose page is only an interstitial.
#[tokio::test]
#[ignore = "needs a database"]
async fn a_forward_code_can_be_released_from_its_manage_page() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];

    support::claim(
        &router,
        &code.id,
        &code.secret,
        "forward",
        serde_json::json!({ "url": "https://example.com" }),
    )
    .await;
    let response = release(&router, code).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let chooser = page(&router, code).await;
    assert!(
        chooser.contains("Recover QR code") && chooser.contains("Website link"),
        "the released forward code did not offer its history:\n{chooser}"
    );
}
