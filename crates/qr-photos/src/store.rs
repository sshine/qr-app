//! The album's rows.
//!
//! Every query is scoped by binding, not just by photo id, so a photo id lifted from
//! one code cannot be read or deleted through another. The capability layer already
//! proved which code the request holds; this is what stops that proof being spent on
//! someone else's box.

use chrono::{DateTime, Utc};
use qr_core::Result;
use serde::Serialize;
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Photo {
    pub id: Uuid,
    pub original_key: String,
    pub view_key: String,
    pub thumb_key: String,
    pub media_type: String,
    pub width: i32,
    pub height: i32,
    pub byte_size: i64,
    pub taken_at: Option<DateTime<Utc>>,
    pub caption: Option<String>,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

pub struct NewPhoto<'a> {
    pub binding_id: Uuid,
    pub original_key: &'a str,
    pub view_key: &'a str,
    pub thumb_key: &'a str,
    pub media_type: &'a str,
    pub width: i32,
    pub height: i32,
    pub byte_size: i64,
    pub taken_at: Option<DateTime<Utc>>,
}

macro_rules! photo_from_row {
    ($row:expr) => {
        Photo {
            id: $row.id,
            original_key: $row.original_key,
            view_key: $row.view_key,
            thumb_key: $row.thumb_key,
            media_type: $row.media_type,
            width: $row.width,
            height: $row.height,
            byte_size: $row.byte_size,
            taken_at: $row.taken_at,
            caption: $row.caption,
            created_at: $row.created_at,
            deleted_at: $row.deleted_at,
        }
    };
}

pub async fn insert<'e, E: PgExecutor<'e>>(ex: E, new: NewPhoto<'_>) -> Result<Photo> {
    // UUIDv7, so ORDER BY id DESC is newest-first with no second sort key and no
    // tiebreak between two uploads landing in the same millisecond.
    let row = sqlx::query!(
        r#"
        INSERT INTO photo_items
            (id, binding_id, original_key, view_key, thumb_key, media_type,
             width, height, byte_size, taken_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, original_key, view_key, thumb_key, media_type, width, height,
                  byte_size, taken_at, caption, created_at, deleted_at
        "#,
        Uuid::now_v7(),
        new.binding_id,
        new.original_key,
        new.view_key,
        new.thumb_key,
        new.media_type,
        new.width,
        new.height,
        new.byte_size,
        new.taken_at,
    )
    .fetch_one(ex)
    .await?;

    Ok(photo_from_row!(row))
}

/// The album, newest first, answered from the partial index.
pub async fn list<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Photo>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, original_key, view_key, thumb_key, media_type, width, height,
               byte_size, taken_at, caption, created_at, deleted_at
        FROM photo_items
        WHERE binding_id = $1
          AND deleted_at IS NULL
          AND ($2::uuid IS NULL OR id < $2)
        ORDER BY id DESC
        LIMIT $3
        "#,
        binding_id,
        before,
        limit,
    )
    .fetch_all(ex)
    .await?;

    Ok(rows.into_iter().map(|row| photo_from_row!(row)).collect())
}

/// What delete put aside, most recently removed first.
pub async fn trash<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<Vec<Photo>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, original_key, view_key, thumb_key, media_type, width, height,
               byte_size, taken_at, caption, created_at, deleted_at
        FROM photo_items
        WHERE binding_id = $1 AND deleted_at IS NOT NULL
        ORDER BY deleted_at DESC
        "#,
        binding_id,
    )
    .fetch_all(ex)
    .await?;

    Ok(rows.into_iter().map(|row| photo_from_row!(row)).collect())
}

/// One photo, including a deleted one, so a thumbnail survives until undo is gone.
pub async fn get<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    id: Uuid,
) -> Result<Option<Photo>> {
    let row = sqlx::query!(
        r#"
        SELECT id, original_key, view_key, thumb_key, media_type, width, height,
               byte_size, taken_at, caption, created_at, deleted_at
        FROM photo_items
        WHERE binding_id = $1 AND id = $2
        "#,
        binding_id,
        id,
    )
    .fetch_optional(ex)
    .await?;

    Ok(row.map(|row| photo_from_row!(row)))
}

/// The thumbnail for a photo by its own id, without a binding in hand.
///
/// The overview is public and its cards point at photos across every label, so it cannot
/// use the per-code media route. The id is a UUIDv7 and the bytes behind it are already
/// served as `public` there; this is the same read, not a wider one.
pub async fn thumb_key<'e, E: PgExecutor<'e>>(ex: E, id: Uuid) -> Result<Option<String>> {
    let row = sqlx::query!("SELECT thumb_key FROM photo_items WHERE id = $1", id,)
        .fetch_optional(ex)
        .await?;
    Ok(row.map(|row| row.thumb_key))
}

/// Soft, because the only confirmation a delete gets is the undo that follows it.
pub async fn soft_delete<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid, id: Uuid) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE photo_items SET deleted_at = now()
        WHERE binding_id = $1 AND id = $2 AND deleted_at IS NULL
        RETURNING id
        "#,
        binding_id,
        id,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

pub async fn restore<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid, id: Uuid) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE photo_items SET deleted_at = NULL
        WHERE binding_id = $1 AND id = $2 AND deleted_at IS NOT NULL
        RETURNING id
        "#,
        binding_id,
        id,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

pub async fn set_caption<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    id: Uuid,
    caption: Option<&str>,
) -> Result<bool> {
    let row = sqlx::query!(
        "UPDATE photo_items SET caption = $3 WHERE binding_id = $1 AND id = $2 RETURNING id",
        binding_id,
        id,
        caption,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

pub async fn count<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<i64> {
    let count = sqlx::query_scalar!(
        "SELECT count(*) FROM photo_items WHERE binding_id = $1 AND deleted_at IS NULL",
        binding_id,
    )
    .fetch_one(ex)
    .await?;
    Ok(count.unwrap_or(0))
}
