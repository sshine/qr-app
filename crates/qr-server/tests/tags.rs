//! Tags on a sticker.
//!
//! The catalog is shared and the tag lives on the code, not on the binding. The two
//! properties worth stating are that a tag created on one phone can be chosen on another,
//! and that emptying a box does not remove the words written on its sticker.

#![allow(clippy::expect_used)]

mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use qr_core::{MintedCode, testing::TestDb};

async fn claim_album(router: &axum::Router, code: &MintedCode) {
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

async fn add(router: &axum::Router, code: &MintedCode, body: &str) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/tags", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request"),
    )
    .await
}

async fn list(router: &axum::Router, code: &MintedCode) -> serde_json::Value {
    let response = support::get(
        router,
        &format!("/c/{}/api/v1/tags", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    serde_json::from_str(&support::text(response).await).expect("json")
}

async fn page(router: &axum::Router, code: &MintedCode, path: &str) -> String {
    let uri = if path.is_empty() {
        format!("/c/{}", code.id)
    } else {
        format!("/c/{}/{path}", code.id)
    };
    let response = support::get(router, &uri, Some(support::cookie(&code.id, &code.secret))).await;
    support::assert_status(&response, StatusCode::OK);
    support::text(response).await
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_tag_created_on_one_sticker_can_be_chosen_on_another() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (first, second) = (&codes[0], &codes[1]);
    claim_album(&router, first).await;
    claim_album(&router, second).await;

    let response = add(&router, first, r#"{"name":"Loft"}"#).await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        parsed["tags"][0]["name"].as_str(),
        Some("Loft"),
        "the tag was not attached: {body}"
    );

    let catalog = list(&router, second).await;
    assert_eq!(
        catalog["catalog"][0]["name"].as_str(),
        Some("Loft"),
        "the second sticker cannot see the shared catalog: {catalog}"
    );
    let id = catalog["catalog"][0]["id"]
        .as_str()
        .expect("catalog id")
        .to_owned();

    let response = add(&router, second, &format!(r#"{{"id":"{id}"}}"#)).await;
    support::assert_status(&response, StatusCode::OK);

    let body = page(&router, second, "").await;
    assert!(
        body.contains("Loft"),
        "the second sticker does not show the tag:\n{body}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tags")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 1, "choosing a tag created a second row");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_tag_name_is_the_same_whatever_its_case() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    claim_album(&router, &codes[0]).await;
    claim_album(&router, &codes[1]).await;

    let response = add(&router, &codes[0], r#"{"name":"Loft"}"#).await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    let id = parsed["tags"][0]["id"].as_str().expect("id").to_owned();

    let response = add(&router, &codes[1], r#"{"name":"loft"}"#).await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        parsed["tags"][0]["id"].as_str(),
        Some(id.as_str()),
        "a different case made a different tag: {body}"
    );
    assert_eq!(
        parsed["tags"][0]["name"].as_str(),
        Some("Loft"),
        "the existing spelling was not kept: {body}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tags")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 1, "case made a second tag row");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_tag_survives_release_and_recovery() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    let response = add(&router, code, r#"{"name":"Garage"}"#).await;
    support::assert_status(&response, StatusCode::OK);

    let response = support::send(
        &router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/release", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let held = qr_core::store::code_tags(db.pool(), &code.id)
        .await
        .expect("lookup");
    assert_eq!(held.len(), 1, "release removed the tag from the sticker");

    let binding = qr_core::store::released_bindings(db.pool(), &code.id)
        .await
        .expect("history")
        .into_iter()
        .next()
        .expect("released binding");
    let response = support::send(
        &router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/recover", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("binding={}", binding.id)))
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, code, "").await;
    assert!(
        body.contains("Garage"),
        "the tag was lost through recovery:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_tag_can_be_removed_and_the_page_works_without_script() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    let response = add(&router, code, r#"{"name":"Depot"}"#).await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    let id = parsed["tags"][0]["id"].as_str().expect("id").to_owned();

    let response = support::send(
        &router,
        Request::builder()
            .method("DELETE")
            .uri(format!("/c/{}/api/v1/tags/{id}", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);

    let body = page(&router, code, "").await;
    assert!(
        !body.contains("Depot"),
        "the removed tag is still on the page:\n{body}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM code_tags")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 0, "removing left the join row in place");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn the_no_script_page_adds_and_removes_tags() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    let response = support::send(
        &router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/tags/add", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("name=Shelf+3"))
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, code, "tags").await;
    assert!(
        body.contains("Shelf 3"),
        "the form did not add the tag:\n{body}"
    );

    let held = qr_core::store::code_tags(db.pool(), &code.id)
        .await
        .expect("lookup");
    let id = held[0].id;

    let response = support::send(
        &router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/tags/{id}/remove", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, code, "tags").await;
    assert!(
        body.contains("No tags yet"),
        "the form did not remove the tag:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn an_empty_or_overlong_tag_name_is_refused() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    for body in [r#"{"name":""}"#, r#"{"name":"   "}"#, r#"{}"#] {
        let response = add(&router, code, body).await;
        support::assert_status(&response, StatusCode::BAD_REQUEST);
    }

    let overlong = format!(r#"{{"name":"{}"}}"#, "x".repeat(41));
    let response = add(&router, code, &overlong).await;
    support::assert_status(&response, StatusCode::BAD_REQUEST);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tags")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 0, "a refused tag was still written");
}
