//! Where a sticker is.
//!
//! The catalog is shared and the tag lives on the code, not on the binding. The two
//! properties worth stating are that a place created on one phone can be chosen on
//! another, and that emptying a box does not move it off its shelf.

#![allow(clippy::expect_used)]

mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use qr_core::{MintedCode, store, testing::TestDb};

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

async fn tag(router: &axum::Router, code: &MintedCode, form: &str) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/location", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .expect("request"),
    )
    .await
}

async fn clear(router: &axum::Router, code: &MintedCode) -> axum::response::Response {
    support::send(
        router,
        Request::builder()
            .method("POST")
            .uri(format!("/c/{}/api/v1/location/clear", code.id))
            .header(header::COOKIE, support::cookie(&code.id, &code.secret))
            .body(Body::empty())
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

#[tokio::test]
#[ignore = "needs a database"]
async fn a_place_created_on_one_sticker_can_be_chosen_on_another() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 2).await;
    let (first, second) = (&codes[0], &codes[1]);
    claim_album(&router, first).await;
    claim_album(&router, second).await;

    let form = "name=Loft%2C+shelf+3&address=The+narrow+part&lat=55.676100&lon=12.568300";
    let response = tag(&router, first, form).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, first).await;
    assert!(
        body.contains("Loft, shelf 3"),
        "the tag did not appear in the control bar:\n{body}"
    );

    // The catalog a script sees, and the id the second sticker can pick.
    let response = support::get(
        &router,
        &format!("/c/{}/api/v1/locations", second.id),
        Some(support::cookie(&second.id, &second.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    let locations = parsed
        .get("locations")
        .and_then(|v| v.as_array())
        .expect("list");
    assert_eq!(locations.len(), 1, "the catalog has {locations:?}");
    let id = locations[0]
        .get("id")
        .and_then(|v| v.as_str())
        .expect("id")
        .to_owned();
    assert_eq!(
        locations[0].get("name").and_then(|v| v.as_str()),
        Some("Loft, shelf 3")
    );

    let response = tag(&router, second, &format!("location={id}")).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, second).await;
    assert!(
        body.contains("Loft, shelf 3"),
        "the second sticker did not take the shared place:\n{body}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM locations")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 1, "choosing a place created a second row");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn coordinates_outside_the_world_are_refused() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    for form in [
        "name=Nowhere&lat=90.1&lon=0",
        "name=Nowhere&lat=0&lon=-181",
        "name=Nowhere&lat=NaN&lon=0",
        "name=&lat=55&lon=12",
    ] {
        let response = tag(&router, code, form).await;
        support::assert_status(&response, StatusCode::BAD_REQUEST);
    }

    // An optional address may be left empty; that is not the same as a missing name.
    let response = tag(&router, code, "name=Nowhere&lat=55&lon=12&address=").await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    let body = page(&router, code).await;
    assert!(
        body.contains("Nowhere"),
        "the page does not show the accepted tag:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn emptying_a_box_does_not_move_it_off_its_shelf() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    let response = tag(&router, code, "name=Garage&lat=55.7&lon=12.5").await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

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

    assert!(
        store::code_location(db.pool(), &code.id)
            .await
            .expect("lookup")
            .is_some(),
        "release removed the location from the sticker"
    );

    let binding = store::released_bindings(db.pool(), &code.id)
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

    let body = page(&router, code).await;
    assert!(
        body.contains("Garage"),
        "the shelf was lost through recovery:\n{body}"
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_tag_can_be_removed() {
    let db = TestDb::create().await.expect("test database");
    let dir = tempfile::tempdir().expect("tempdir");
    let router = support::router(&db, &dir).await;
    let codes = support::mint(&db, 1).await;
    let code = &codes[0];
    claim_album(&router, code).await;

    tag(&router, code, "name=Garage&lat=55.7&lon=12.5").await;
    let response = clear(&router, code).await;
    support::assert_status(&response, StatusCode::SEE_OTHER);

    assert!(
        store::code_location(db.pool(), &code.id)
            .await
            .expect("lookup")
            .is_none(),
        "clearing left the tag in place"
    );
    let response = support::get(
        &router,
        &format!("/c/{}/location", code.id),
        Some(support::cookie(&code.id, &code.secret)),
    )
    .await;
    support::assert_status(&response, StatusCode::OK);
    let body = support::text(response).await;
    assert!(
        body.contains("Not tagged yet"),
        "the page still shows a tag:\n{body}"
    );
}
