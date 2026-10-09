//! The storage layer, against a real Postgres.
//!
//! Ignored rather than feature-gated, so `--all-features` cannot switch these on where
//! there is no server. `nix flake check` starts one in the sandbox; locally it is
//! `just db-up` and `just test-db`.

#![cfg(feature = "testing")]
#![allow(clippy::expect_used)]

use qr_core::{
    CodeId, EventKind, Secret,
    ids::{DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN},
    store,
    testing::TestDb,
};
use serde_json::json;

async fn db() -> TestDb {
    TestDb::create().await.expect("test database")
}

async fn one_code(db: &TestDb) -> (CodeId, Secret) {
    let mut conn = db.pool().acquire().await.expect("connection");
    let minted = store::mint_codes(
        &mut conn,
        1,
        DEFAULT_CODE_ID_LEN,
        DEFAULT_SECRET_LEN,
        None,
        None,
        &json!({}),
    )
    .await
    .expect("mint");
    let code = minted.into_iter().next().expect("one code");
    (code.id, code.secret)
}

#[tokio::test]
#[ignore = "needs a database"]
async fn minting_gives_distinct_ids_and_secrets_that_verify() {
    let db = db().await;
    let mut conn = db.pool().acquire().await.expect("connection");

    let minted = store::mint_codes(
        &mut conn,
        32,
        DEFAULT_CODE_ID_LEN,
        DEFAULT_SECRET_LEN,
        None,
        Some("photo-album"),
        &json!({"from": "a test"}),
    )
    .await
    .expect("mint");

    assert_eq!(minted.len(), 32);
    let ids: std::collections::BTreeSet<_> = minted.iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), 32, "two codes share an id");

    for m in &minted {
        let found = store::redeem(db.pool(), &m.id, &m.secret)
            .await
            .expect("redeem")
            .expect("the secret should be accepted");
        assert_eq!(found.intended_handler.as_deref(), Some("photo-album"));
        assert_eq!(found.intended_config, json!({"from": "a test"}));
    }
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_wrong_secret_an_unknown_id_and_a_revoked_code_all_come_back_empty() {
    let db = db().await;
    let (id, secret) = one_code(&db).await;

    let wrong = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
    assert!(
        store::redeem(db.pool(), &id, &wrong)
            .await
            .expect("redeem")
            .is_none()
    );

    let unknown = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
    assert!(
        store::redeem(db.pool(), &unknown, &secret)
            .await
            .expect("redeem")
            .is_none()
    );

    // The right secret still works until the code is revoked, and never after.
    assert!(
        store::redeem(db.pool(), &id, &secret)
            .await
            .expect("redeem")
            .is_some()
    );
    store::revoke_code(db.pool(), &id).await.expect("revoke");
    assert!(
        store::redeem(db.pool(), &id, &secret)
            .await
            .expect("redeem")
            .is_none()
    );
}

#[tokio::test]
#[ignore = "needs a database"]
async fn the_first_scan_is_stamped_exactly_once() {
    let db = db().await;
    let (id, _) = one_code(&db).await;

    assert!(store::mark_first_seen(db.pool(), &id).await.expect("first"));
    assert!(
        !store::mark_first_seen(db.pool(), &id)
            .await
            .expect("second"),
        "a second scan should not look like a first"
    );

    let code = store::code(db.pool(), &id)
        .await
        .expect("code")
        .expect("exists");
    assert!(code.first_seen_at.is_some());
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_code_can_hold_only_one_live_binding() {
    let db = db().await;
    let (id, _) = one_code(&db).await;

    let first = store::insert_binding(db.pool(), &id, "photo-album", &json!({}), None)
        .await
        .expect("bind");

    // Two phones scanning one sticker at the same moment both reach here.
    let second = store::insert_binding(db.pool(), &id, "forward", &json!({}), None).await;
    let err = second.expect_err("the second claim should lose");
    assert!(err.is_conflict(), "wrong error: {err}");

    let active = store::active_binding(db.pool(), &id)
        .await
        .expect("active")
        .expect("one binding");
    assert_eq!(active.id, first.id);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn releasing_a_code_frees_it_without_destroying_what_was_there() {
    let db = db().await;
    let (id, _) = one_code(&db).await;

    let first = store::insert_binding(db.pool(), &id, "photo-album", &json!({}), Some("winter"))
        .await
        .expect("bind");
    store::release_binding(db.pool(), first.id)
        .await
        .expect("release");

    assert!(
        store::active_binding(db.pool(), &id)
            .await
            .expect("active")
            .is_none()
    );

    let second = store::insert_binding(db.pool(), &id, "forward", &json!({}), None)
        .await
        .expect("rebind");
    assert_ne!(second.id, first.id, "the old binding should still exist");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_binding_config_and_title_can_be_changed_in_place() {
    let db = db().await;
    let (id, _) = one_code(&db).await;
    let binding = store::insert_binding(db.pool(), &id, "forward", &json!({}), None)
        .await
        .expect("bind");

    store::set_binding_config(
        db.pool(),
        binding.id,
        &json!({"url": "https://example.com"}),
    )
    .await
    .expect("config");
    store::set_binding_title(db.pool(), binding.id, Some("the poster"))
        .await
        .expect("title");

    let active = store::active_binding(db.pool(), &id)
        .await
        .expect("active")
        .expect("bound");
    assert_eq!(active.config["url"], "https://example.com");
    assert_eq!(active.title.as_deref(), Some("the poster"));
}

#[tokio::test]
#[ignore = "needs a database"]
async fn registering_the_same_blob_twice_is_a_no_op() {
    let db = db().await;
    let key = "a".repeat(64);

    store::register_blob(db.pool(), &key, 12, "image/jpeg")
        .await
        .expect("first");
    store::register_blob(db.pool(), &key, 12, "image/jpeg")
        .await
        .expect("second");

    let record = store::blob(db.pool(), &key)
        .await
        .expect("blob")
        .expect("exists");
    assert_eq!(record.byte_size, 12);
}

#[tokio::test]
#[ignore = "needs a database"]
async fn events_accumulate_and_can_be_counted_per_code() {
    let db = db().await;
    let (id, _) = one_code(&db).await;

    for _ in 0..3 {
        store::record_event(
            db.pool(),
            EventKind::ForwardFollowed,
            Some(&id),
            None,
            &json!({}),
        )
        .await
        .expect("event");
    }
    store::record_event(
        db.pool(),
        EventKind::CodeFirstSeen,
        Some(&id),
        None,
        &json!({}),
    )
    .await
    .expect("event");

    let followed = store::count_events(db.pool(), EventKind::ForwardFollowed, &id)
        .await
        .expect("count");
    assert_eq!(followed, 3, "the scan counter should not count other kinds");
}

#[tokio::test]
#[ignore = "needs a database"]
async fn a_sheet_owns_its_codes_and_can_record_the_pdf_it_rendered_to() {
    let db = db().await;
    let mut tx = db.store().begin().await.expect("begin");

    let sheet = store::insert_sheet(
        &mut *tx,
        store::NewSheet {
            id: uuid::Uuid::now_v7(),
            code_count: 21,
            base_url: "https://qr.shine.town",
            preset: "relief-t7160",
            layout: &json!({"cols": 3}),
            handler_id: Some("photo-album"),
            handler_config: &json!({}),
            note: Some("garage"),
        },
    )
    .await
    .expect("sheet");

    let minted = store::mint_codes(
        &mut tx,
        21,
        DEFAULT_CODE_ID_LEN,
        DEFAULT_SECRET_LEN,
        Some(sheet.id),
        sheet.handler_id.as_deref(),
        &sheet.handler_config,
    )
    .await
    .expect("mint");
    assert_eq!(minted.len(), 21);

    let key = "b".repeat(64);
    store::register_blob(&mut *tx, &key, 4096, "application/pdf")
        .await
        .expect("blob");
    store::attach_sheet_pdf(&mut *tx, sheet.id, &key, None)
        .await
        .expect("attach");

    tx.commit().await.expect("commit");

    let first = minted.first().expect("a code");
    let code = store::code(db.pool(), &first.id)
        .await
        .expect("code")
        .expect("exists");
    assert_eq!(code.sheet_id, Some(sheet.id));
    assert_eq!(code.intended_handler.as_deref(), Some("photo-album"));
}
