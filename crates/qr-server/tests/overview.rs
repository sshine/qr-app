//! The one page that lists every label.
//!
//! It is deliberately not a capability route, so the properties worth stating are that
//! hiding a card does not stop the sticker, that one card carries a repeated deletion
//! instead of becoming several, and that the filters select the labels they say they do.

#![allow(clippy::expect_used)]

mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use qr_core::{CodeId, MintedCode, testing::TestDb};

/// A one-pixel PNG, made here rather than checked in as a fixture.
fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 40, 20, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

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

async fn upload(router: &axum::Router, code: &MintedCode) {
    let boundary = "----qr-overview-test";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"photo\"; filename=\"box.png\"\r\n\
             Content-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&png());
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let response = support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/h/photo-album/photos", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
}

async fn page(router: &axum::Router, uri: &str) -> String {
    let response = support::get(router, uri, None).await;
    support::assert_status(&response, StatusCode::OK);
    support::text(response).await
}

async fn post(router: &axum::Router, uri: &str, form: &str) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .expect("request"),
    )
    .await
}

async fn post_as(
    router: &axum::Router,
    code: &MintedCode,
    uri: &str,
    form: &str,
) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .expect("request"),
    )
    .await
}

fn grouped(code: &CodeId) -> String {
    code.grouped()
}

#[tokio::test]
#[ignore = "needs a database"]
async fn hiding_a_label_keeps_the_sticker_working_and_groups_repeated_hides() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    album(&router, code).await;
    let shown = grouped(&code.id);

    let body = page(&router, "/kiksmiks").await;
    assert!(
        body.contains(&shown),
        "the minted label is not listed:\n{body}"
    );

    for _ in 0..1 {
        let response = post(&router, &format!("/kiksmiks/{}/delete", code.id), "back=").await;
        support::assert_status(&response, StatusCode::SEE_OTHER);

        let body = page(&router, "/kiksmiks").await;
        assert!(
            !body.contains(&shown),
            "a hidden label is still in the default view:\n{body}"
        );

        let body = page(&router, "/kiksmiks?deleted=1").await;
        assert!(
            body.contains(&shown),
            "a hidden label is missing from the deleted view:\n{body}"
        );

        let response = post(&router, &format!("/kiksmiks/{}/restore", code.id), "back=").await;
        support::assert_status(&response, StatusCode::SEE_OTHER);
    }

    // Deleting does not revoke: the sticker still opens the album it was bound to.
    let response = support::get(
        &router,
        &format!("/c/{}", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);

    // The second hide is not a second card: the same one carries its history.
    post(&router, &format!("/kiksmiks/{}/delete", code.id), "back=").await;
    let body = page(&router, "/kiksmiks?deleted=1").await;
    assert!(
        body.contains("deleted 2 times"),
        "the repeated hide was not grouped onto one card:\n{body}"
    );
    let cards = body.matches("class=\"label-card deleted\"").count();
    assert_eq!(cards, 1, "the sticker appears as {cards} cards");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn the_overview_filters_by_tag_place_and_text() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (first, second) = (&codes[0], &codes[1]);
    album(&router, first).await;
    album(&router, second).await;

    // A unique tag on the first, a unique place on the first, and nothing on the second.
    let response = support::send(
        &router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/tags", first.id))
            .header(header::COOKIE, support::cookie(&first.id, &first.secret))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "name": "Overview loft" }).to_string(),
            ))
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    let tag = parsed["tags"][0]["id"].as_str().expect("tag id").to_owned();

    let response = post_as(
        &router,
        first,
        &format!("/c/{}/api/v1/location", first.id),
        "name=Overview+garage&lat=55.7&lon=12.5",
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
    let place: String =
        sqlx::query_scalar("SELECT id::text FROM locations WHERE name = 'Overview garage'")
            .fetch_one(db.pool())
            .await
            .expect("place");

    let first_code = grouped(&first.id);
    let second_code = grouped(&second.id);

    let body = page(&router, &format!("/kiksmiks?tag_{tag}=1")).await;
    assert!(
        body.contains(&first_code) && !body.contains(&second_code),
        "the tag filter selected the wrong labels:\n{body}"
    );

    let body = page(&router, &format!("/kiksmiks?loc_{place}=1")).await;
    assert!(
        body.contains(&first_code) && !body.contains(&second_code),
        "the place filter selected the wrong labels:\n{body}"
    );

    let body = page(&router, "/kiksmiks?q=Overview+loft").await;
    assert!(
        body.contains(&first_code) && !body.contains(&second_code),
        "the text filter selected the wrong labels:\n{body}"
    );

    // Categories combine with AND: a tag that matches and a place that does not is empty.
    let other: String = sqlx::query_scalar("SELECT id::text FROM locations LIMIT 1")
        .fetch_one(db.pool())
        .await
        .expect("place");
    let body = page(
        &router,
        &format!("/kiksmiks?tag_{tag}=1&loc_{other}=1&q=does-not-match"),
    )
    .await;
    assert!(
        !body.contains(&first_code) && !body.contains(&second_code),
        "a filter that matches nothing still listed labels:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_printed_label_is_hidden_until_it_is_used() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    let shown = grouped(&code.id);

    let body = page(&router, "/kiksmiks").await;
    assert!(
        !body.contains(&shown),
        "a code that was only printed is listed as a label:\n{body}"
    );

    album(&router, code).await;
    let body = page(&router, "/kiksmiks").await;
    assert!(
        body.contains(&shown),
        "a used label disappeared from the gallery:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_label_page_edits_the_title_tags_and_place_the_gallery_searches() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    album(&router, code).await;

    let body = page(&router, &format!("/kiksmiks/{}", code.id)).await;
    for expected in ["Title", "Tags", "Place", "Open this label"] {
        assert!(
            body.contains(expected),
            "the label page is missing {expected}:\n{body}"
        );
    }

    let binding: uuid::Uuid = sqlx::query_scalar("SELECT id FROM bindings WHERE code_id = $1")
        .bind(code.id.as_str())
        .fetch_one(db.pool())
        .await
        .expect("binding");

    let response = post(
        &router,
        &format!("/kiksmiks/{}/title", code.id),
        &format!("binding={binding}&title=Detail+box&back="),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let response = post(
        &router,
        &format!("/kiksmiks/{}/tags", code.id),
        "name=Detail+loft&back=",
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let response = post(
        &router,
        &format!("/kiksmiks/{}/place", code.id),
        "name=Detail+shelf&lat=55.7&lon=12.5&back=",
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let shown = grouped(&code.id);
    for uri in [
        "/kiksmiks?q=Detail+box".to_owned(),
        "/kiksmiks?q=Detail+loft".to_owned(),
        "/kiksmiks?q=Detail+shelf".to_owned(),
    ] {
        let body = page(&router, &uri).await;
        assert!(
            body.contains(&shown),
            "search {uri} lost the label:\n{body}"
        );
    }

    let tag: String = sqlx::query_scalar("SELECT id::text FROM tags WHERE name = 'Detail loft'")
        .fetch_one(db.pool())
        .await
        .expect("tag");
    let body = page(&router, &format!("/kiksmiks?tag_{tag}=1")).await;
    assert!(
        body.contains(&shown),
        "the tag filter lost the label:\n{body}"
    );

    let place: String =
        sqlx::query_scalar("SELECT id::text FROM locations WHERE name = 'Detail shelf'")
            .fetch_one(db.pool())
            .await
            .expect("place");
    let body = page(&router, &format!("/kiksmiks?loc_{place}=1")).await;
    assert!(
        body.contains(&shown),
        "the place filter lost the label:\n{body}"
    );

    // Release keeps the card and its history, and the page offers it back.
    let response = post(&router, &format!("/kiksmiks/{}/release", code.id), "back=").await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
    let body = page(&router, &format!("/kiksmiks/{}", code.id)).await;
    assert!(
        body.contains("Previous uses") && body.contains("Recover"),
        "a released label did not offer its history back:\n{body}"
    );

    let response = post(
        &router,
        &format!("/kiksmiks/{}/recover", code.id),
        &format!("binding={binding}&back="),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);
    let body = page(&router, &format!("/kiksmiks/{}", code.id)).await;
    assert!(
        body.contains("Open this label"),
        "recovery did not reactivate"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn an_unchecksummed_code_cannot_reach_the_gallery_or_its_routes() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;

    // A row minted before the cut-over. The migration deletes these in production;
    // inserting one here proves the routes refuse it even if a database still has it.
    sqlx::query("INSERT INTO codes (id, secret_hash) VALUES ('ABC12345', $1)")
        .bind(&[0u8; 32][..])
        .execute(db.pool())
        .await
        .expect("legacy code");

    let response = support::get(&router, "/kiksmiks/ABC12345", None).await;
    support::assert_status(&response, StatusCode::NOT_FOUND);

    let response = support::get(
        &router,
        "/c/ABC12345/api/v1/state",
        Some("qc_ABC12345=".to_owned() + "0123456789ABCDEF"),
    )
    .await;
    support::assert_status(&response, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_photo_label_shows_its_top_photo_and_the_rest_as_a_count() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    album(&router, code).await;

    for _ in 0..3 {
        upload(&router, code).await;
    }

    let top: uuid::Uuid = sqlx::query_scalar("SELECT id FROM photo_items ORDER BY id DESC LIMIT 1")
        .fetch_one(db.pool())
        .await
        .expect("top photo");

    let body = page(&router, "/kiksmiks").await;
    assert!(
        body.contains(&format!("/kiksmiks/photo/{top}/thumb")),
        "the card does not use the top photo's thumbnail:\n{body}"
    );
    assert!(
        body.contains("+2"),
        "the extra photos are not counted:\n{body}"
    );

    let response = support::get(&router, &format!("/kiksmiks/photo/{top}/thumb"), None).await;
    support::assert_status(&response, StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/jpeg"),
        "the overview thumbnail is not served as a jpeg"
    );
}
