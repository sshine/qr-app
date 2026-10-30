//! Reading and writing the schema.
//!
//! Most of this is free functions over an executor rather than methods on a connection,
//! so a caller that needs several of them inside one transaction can have that without
//! this module knowing what the transaction is for. `Store` is the pool-held convenience
//! on top, for the cases that are a single statement.

use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    CodeId, Error, Result, Secret, SecretHash, decoy_hash,
    model::{
        Binding, BlobRecord, Code, CodeLink, EventKind, Location, MintedCode, NewLocation, Sheet,
        Tag,
    },
};

/// How many times minting retries a colliding id before giving up.
///
/// A collision is a lost race against the primary key, not a shortage of ids: at the
/// default length the space is a thousand billion. Three retries turns a run of bad luck
/// into a slower insert and a genuine exhaustion into an error rather than a hang.
const MINT_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn begin(&self) -> Result<Transaction<'static, Postgres>> {
        Ok(self.pool.begin().await?)
    }

    /// Applies every migration this binary carries.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| Error::Database(sqlx::Error::Migrate(Box::new(e))))
    }

    /// Can this connection still answer? Used by the readiness probe.
    pub async fn probe(&self) -> Result<()> {
        sqlx::query_scalar!("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}

/// What a sheet is created from.
///
/// A struct rather than eight arguments: the three optional strings are interchangeable
/// at a call site and a transposed pair would be a sheet whose note is its preset.
pub struct NewSheet<'a> {
    pub id: Uuid,
    pub code_count: i32,
    pub base_url: &'a str,
    pub preset: &'a str,
    pub layout: &'a Json,
    /// What every code on the sheet becomes on first scan, or `None` for the chooser.
    pub handler_id: Option<&'a str>,
    pub handler_config: &'a Json,
    pub note: Option<&'a str>,
}

/// Inserts a sheet row.
pub async fn insert_sheet<'e, E: PgExecutor<'e>>(ex: E, new: NewSheet<'_>) -> Result<Sheet> {
    let NewSheet {
        id,
        code_count,
        base_url,
        preset,
        layout,
        handler_id,
        handler_config,
        note,
    } = new;

    let row = sqlx::query!(
        r#"
        INSERT INTO sheets
            (id, code_count, base_url, preset, layout, handler_id, handler_config, note)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id, created_at, code_count, base_url, preset, layout,
                  handler_id, handler_config, note, pdf_key, pdf_expires_at
        "#,
        id,
        code_count,
        base_url,
        preset,
        layout,
        handler_id,
        handler_config,
        note,
    )
    .fetch_one(ex)
    .await?;

    Ok(Sheet {
        id: row.id,
        created_at: row.created_at,
        code_count: row.code_count,
        base_url: row.base_url,
        preset: row.preset,
        layout: row.layout,
        handler_id: row.handler_id,
        handler_config: row.handler_config,
        note: row.note,
        pdf_key: row.pdf_key,
        pdf_expires_at: row.pdf_expires_at,
    })
}

/// Records the rendered PDF against its sheet, so the sheet can be reprinted.
pub async fn attach_sheet_pdf<'e, E: PgExecutor<'e>>(
    ex: E,
    sheet_id: Uuid,
    pdf_key: &str,
    expires_at: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query!(
        "UPDATE sheets SET pdf_key = $2, pdf_expires_at = $3 WHERE id = $1",
        sheet_id,
        pdf_key,
        expires_at,
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// Mints `count` codes, retrying any that lose a race on the primary key.
///
/// Takes a connection rather than an executor because each insert is its own statement
/// and a retry needs the same connection; pass the transaction a sheet is being built in
/// so a failure halfway leaves neither sheet nor codes.
pub async fn mint_codes(
    conn: &mut sqlx::PgConnection,
    count: usize,
    id_len: usize,
    secret_len: usize,
    sheet_id: Option<Uuid>,
    intended_handler: Option<&str>,
    intended_config: &Json,
) -> Result<Vec<MintedCode>> {
    let mut minted = Vec::with_capacity(count);

    for _ in 0..count {
        let mut attempt = 0;
        loop {
            let id = CodeId::generate(id_len)?;
            let secret = Secret::generate(secret_len)?;
            let hash = secret.hash();

            let inserted = sqlx::query!(
                r#"
                INSERT INTO codes
                    (id, sheet_id, secret_hash, intended_handler, intended_config)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (id) DO NOTHING
                RETURNING id
                "#,
                id.as_str(),
                sheet_id,
                &hash.as_bytes()[..],
                intended_handler,
                intended_config,
            )
            .fetch_optional(&mut *conn)
            .await?;

            if inserted.is_some() {
                minted.push(MintedCode { id, secret });
                break;
            }

            attempt += 1;
            if attempt >= MINT_ATTEMPTS {
                return Err(Error::Conflict("code id"));
            }
        }
    }

    Ok(minted)
}

/// Fetches a code by id.
pub async fn code<'e, E: PgExecutor<'e>>(ex: E, id: &CodeId) -> Result<Option<Code>> {
    let row = sqlx::query!(
        r#"
        SELECT id, sheet_id, secret_hash, intended_handler, intended_config,
               created_at, first_seen_at, revoked_at, deleted_at
        FROM codes WHERE id = $1
        "#,
        id.as_str(),
    )
    .fetch_optional(ex)
    .await?;

    row.map(|row| {
        let hash: [u8; 32] = row
            .secret_hash
            .try_into()
            .map_err(|_| Error::Invalid("a stored secret hash is not 32 bytes".to_owned()))?;
        Ok(Code {
            id: CodeId::parse(&row.id)?,
            sheet_id: row.sheet_id,
            secret_hash: SecretHash::from_bytes(hash),
            intended_handler: row.intended_handler,
            intended_config: row.intended_config,
            created_at: row.created_at,
            first_seen_at: row.first_seen_at,
            revoked_at: row.revoked_at,
            deleted_at: row.deleted_at,
        })
    })
    .transpose()
}

/// Checks a capability secret against a code.
///
/// A miss still verifies, against a hash no secret produces, so a nonexistent id does
/// not answer faster than a real one. Without that the id space is an inventory of how
/// many boxes exist, readable with a stopwatch.
pub async fn redeem<'e, E: PgExecutor<'e>>(
    ex: E,
    id: &CodeId,
    secret: &Secret,
) -> Result<Option<Code>> {
    let found = code(ex, id).await?;

    let Some(found) = found else {
        let _ = decoy_hash().verify(secret);
        return Ok(None);
    };

    if !found.secret_hash.verify(secret) || found.is_revoked() {
        return Ok(None);
    }

    Ok(Some(found))
}

/// Stamps the first scan, and reports whether this was it.
pub async fn mark_first_seen<'e, E: PgExecutor<'e>>(ex: E, id: &CodeId) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE codes SET first_seen_at = now()
        WHERE id = $1 AND first_seen_at IS NULL
        RETURNING id
        "#,
        id.as_str(),
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

pub async fn revoke_code<'e, E: PgExecutor<'e>>(ex: E, id: &CodeId) -> Result<()> {
    sqlx::query!(
        "UPDATE codes SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        id.as_str(),
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// Hides a label from the overview. `false` means it was already hidden.
pub async fn soft_delete_code<'e, E: PgExecutor<'e>>(ex: E, id: &CodeId) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE codes SET deleted_at = now()
        WHERE id = $1 AND deleted_at IS NULL
        RETURNING id
        "#,
        id.as_str(),
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

/// Shows a hidden label again. `false` means it was not hidden.
pub async fn restore_code<'e, E: PgExecutor<'e>>(ex: E, id: &CodeId) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE codes SET deleted_at = NULL
        WHERE id = $1 AND deleted_at IS NOT NULL
        RETURNING id
        "#,
        id.as_str(),
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

/// The binding a code currently has, if any.
pub async fn active_binding<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
) -> Result<Option<Binding>> {
    let row = sqlx::query!(
        r#"
        SELECT id, code_id, handler_id, config, title, created_at, released_at
        FROM bindings
        WHERE code_id = $1 AND released_at IS NULL
        "#,
        code_id.as_str(),
    )
    .fetch_optional(ex)
    .await?;

    row.map(|row| {
        Ok(Binding {
            id: row.id,
            code_id: CodeId::parse(&row.code_id)?,
            handler_id: row.handler_id,
            config: row.config,
            title: row.title,
            created_at: row.created_at,
            released_at: row.released_at,
        })
    })
    .transpose()
}

/// Binds a code to a handler.
///
/// Relies on the partial unique index to settle a race rather than checking first: two
/// phones scanning one sticker at the same moment both reach here, and only one row can
/// exist. The loser gets a conflict and reloads into the winner's binding.
pub async fn insert_binding<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
    handler_id: &str,
    config: &Json,
    title: Option<&str>,
) -> Result<Binding> {
    let id = Uuid::now_v7();
    let row = sqlx::query!(
        r#"
        INSERT INTO bindings (id, code_id, handler_id, config, title)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, code_id, handler_id, config, title, created_at, released_at
        "#,
        id,
        code_id.as_str(),
        handler_id,
        config,
        title,
    )
    .fetch_one(ex)
    .await?;

    Ok(Binding {
        id: row.id,
        code_id: CodeId::parse(&row.code_id)?,
        handler_id: row.handler_id,
        config: row.config,
        title: row.title,
        created_at: row.created_at,
        released_at: row.released_at,
    })
}

pub async fn release_binding<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<()> {
    sqlx::query!(
        "UPDATE bindings SET released_at = now() WHERE id = $1 AND released_at IS NULL",
        binding_id,
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// A binding by id, but only if it belongs to this code.
///
/// Scoped rather than fetched and compared, so an id from another code's history is
/// indistinguishable from an id that never existed.
pub async fn binding<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
    binding_id: Uuid,
) -> Result<Option<Binding>> {
    let row = sqlx::query!(
        r#"
        SELECT id, code_id, handler_id, config, title, created_at, released_at
        FROM bindings
        WHERE id = $1 AND code_id = $2
        "#,
        binding_id,
        code_id.as_str(),
    )
    .fetch_optional(ex)
    .await?;

    row.map(|row| {
        Ok(Binding {
            id: row.id,
            code_id: CodeId::parse(&row.code_id)?,
            handler_id: row.handler_id,
            config: row.config,
            title: row.title,
            created_at: row.created_at,
            released_at: row.released_at,
        })
    })
    .transpose()
}

/// Every use of this code, most recently released first.
///
/// The rows are kept indefinitely, which is what makes recovery possible; the list is
/// small because a sticker only changes its mind when a box is emptied and reused.
pub async fn released_bindings<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
) -> Result<Vec<Binding>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, code_id, handler_id, config, title, created_at, released_at
        FROM bindings
        WHERE code_id = $1 AND released_at IS NOT NULL
        ORDER BY released_at DESC
        "#,
        code_id.as_str(),
    )
    .fetch_all(ex)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(Binding {
                id: row.id,
                code_id: CodeId::parse(&row.code_id)?,
                handler_id: row.handler_id,
                config: row.config,
                title: row.title,
                created_at: row.created_at,
                released_at: row.released_at,
            })
        })
        .collect()
}

/// Undoes a release. `false` means the row was not released, or is not there.
///
/// The partial unique index is what refuses a recovery that races a claim: both cannot
/// leave the code with an active binding.
pub async fn recover_binding<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        UPDATE bindings SET released_at = NULL
        WHERE id = $1 AND released_at IS NOT NULL
        RETURNING id
        "#,
        binding_id,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

pub async fn set_binding_config<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    config: &Json,
) -> Result<()> {
    sqlx::query!(
        "UPDATE bindings SET config = $2 WHERE id = $1",
        binding_id,
        config
    )
    .execute(ex)
    .await?;
    Ok(())
}

pub async fn set_binding_title<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    title: Option<&str>,
) -> Result<()> {
    sqlx::query!(
        "UPDATE bindings SET title = $2 WHERE id = $1",
        binding_id,
        title
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// Writes the delegation a link binding carries.
///
/// Inserted inside the transaction that creates the binding, so there is never a claimed
/// link with nothing to follow. The target secret is the capability the link was made
/// with; it is stored rather than hashed because the redirect has to present it again.
pub async fn insert_link<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
    target: &CodeId,
    secret: &Secret,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO code_links (binding_id, target_code_id, target_secret)
        VALUES ($1, $2, $3)
        "#,
        binding_id,
        target.as_str(),
        secret.expose(),
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// The code and secret a link stands in for.
pub async fn link<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<Option<CodeLink>> {
    let row = sqlx::query!(
        "SELECT target_code_id, target_secret FROM code_links WHERE binding_id = $1",
        binding_id,
    )
    .fetch_optional(ex)
    .await?;

    row.map(|row| {
        Ok(CodeLink {
            target: CodeId::parse(&row.target_code_id)?,
            secret: Secret::parse(&row.target_secret)?,
        })
    })
    .transpose()
}

/// The shared catalog, alphabetically.
pub async fn locations<'e, E: PgExecutor<'e>>(ex: E) -> Result<Vec<Location>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, address, lat, lon, created_at
        FROM locations
        ORDER BY lower(name), created_at
        "#,
    )
    .fetch_all(ex)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Location {
            id: row.id,
            name: row.name,
            address: row.address,
            lat: row.lat,
            lon: row.lon,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn location<'e, E: PgExecutor<'e>>(ex: E, id: Uuid) -> Result<Option<Location>> {
    let row = sqlx::query!(
        "SELECT id, name, address, lat, lon, created_at FROM locations WHERE id = $1",
        id,
    )
    .fetch_optional(ex)
    .await?;

    Ok(row.map(|row| Location {
        id: row.id,
        name: row.name,
        address: row.address,
        lat: row.lat,
        lon: row.lon,
        created_at: row.created_at,
    }))
}

/// Adds a place to the catalog.
pub async fn insert_location<'e, E: PgExecutor<'e>>(ex: E, new: NewLocation) -> Result<Location> {
    let row = sqlx::query!(
        r#"
        INSERT INTO locations (id, name, address, lat, lon)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name, address, lat, lon, created_at
        "#,
        Uuid::now_v7(),
        new.name,
        new.address,
        new.lat,
        new.lon,
    )
    .fetch_one(ex)
    .await?;

    Ok(Location {
        id: row.id,
        name: row.name,
        address: row.address,
        lat: row.lat,
        lon: row.lon,
        created_at: row.created_at,
    })
}

/// Where this code is, if it has been tagged.
pub async fn code_location<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
) -> Result<Option<Location>> {
    let row = sqlx::query!(
        r#"
        SELECT l.id, l.name, l.address, l.lat, l.lon, l.created_at
        FROM codes c
        JOIN locations l ON l.id = c.location_id
        WHERE c.id = $1
        "#,
        code_id.as_str(),
    )
    .fetch_optional(ex)
    .await?;

    Ok(row.map(|row| Location {
        id: row.id,
        name: row.name,
        address: row.address,
        lat: row.lat,
        lon: row.lon,
        created_at: row.created_at,
    }))
}

/// Tags the code, or clears the tag with `None`.
pub async fn set_code_location<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
    location: Option<Uuid>,
) -> Result<()> {
    sqlx::query!(
        "UPDATE codes SET location_id = $2 WHERE id = $1",
        code_id.as_str(),
        location,
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// The shared tag catalog, alphabetically.
pub async fn tags<'e, E: PgExecutor<'e>>(ex: E) -> Result<Vec<Tag>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, created_at
        FROM tags
        ORDER BY lower(name), created_at
        "#,
    )
    .fetch_all(ex)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Tag {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn tag<'e, E: PgExecutor<'e>>(ex: E, id: Uuid) -> Result<Option<Tag>> {
    let row = sqlx::query!("SELECT id, name, created_at FROM tags WHERE id = $1", id,)
        .fetch_optional(ex)
        .await?;

    Ok(row.map(|row| Tag {
        id: row.id,
        name: row.name,
        created_at: row.created_at,
    }))
}

/// The tag with this name, created if it did not exist.
///
/// Case-insensitive, and returns the existing spelling rather than the one just typed:
/// two people tagging the same box should not make "Loft" and "loft" different tags.
pub async fn tag_named<'e, E: PgExecutor<'e>>(ex: E, name: &str) -> Result<Tag> {
    let row = sqlx::query!(
        r#"
        INSERT INTO tags (id, name) VALUES ($1, $2)
        ON CONFLICT (lower(name)) DO UPDATE SET name = tags.name
        RETURNING id, name, created_at
        "#,
        Uuid::now_v7(),
        name,
    )
    .fetch_one(ex)
    .await?;

    Ok(Tag {
        id: row.id,
        name: row.name,
        created_at: row.created_at,
    })
}

/// The tags a sticker carries, alphabetically.
pub async fn code_tags<'e, E: PgExecutor<'e>>(ex: E, code_id: &CodeId) -> Result<Vec<Tag>> {
    let rows = sqlx::query!(
        r#"
        SELECT t.id, t.name, t.created_at
        FROM code_tags ct
        JOIN tags t ON t.id = ct.tag_id
        WHERE ct.code_id = $1
        ORDER BY lower(t.name), t.created_at
        "#,
        code_id.as_str(),
    )
    .fetch_all(ex)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Tag {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        })
        .collect())
}

/// Attaches a tag. `false` means it was already there.
pub async fn add_code_tag<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
    tag_id: Uuid,
) -> Result<bool> {
    let row = sqlx::query!(
        r#"
        INSERT INTO code_tags (code_id, tag_id) VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        RETURNING code_id
        "#,
        code_id.as_str(),
        tag_id,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

/// Removes a tag. `false` means the sticker did not carry it.
pub async fn remove_code_tag<'e, E: PgExecutor<'e>>(
    ex: E,
    code_id: &CodeId,
    tag_id: Uuid,
) -> Result<bool> {
    let row = sqlx::query!(
        "DELETE FROM code_tags WHERE code_id = $1 AND tag_id = $2 RETURNING code_id",
        code_id.as_str(),
        tag_id,
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.is_some())
}

/// Records a blob's metadata. Idempotent, because the key is the content.
pub async fn register_blob<'e, E: PgExecutor<'e>>(
    ex: E,
    key: &str,
    byte_size: i64,
    media_type: &str,
) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO blobs (key, byte_size, media_type) VALUES ($1, $2, $3)
        ON CONFLICT (key) DO NOTHING
        "#,
        key,
        byte_size,
        media_type,
    )
    .execute(ex)
    .await?;
    Ok(())
}

pub async fn blob<'e, E: PgExecutor<'e>>(ex: E, key: &str) -> Result<Option<BlobRecord>> {
    let row = sqlx::query!(
        "SELECT key, byte_size, media_type, created_at FROM blobs WHERE key = $1",
        key
    )
    .fetch_optional(ex)
    .await?;

    Ok(row.map(|row| BlobRecord {
        key: row.key,
        byte_size: row.byte_size,
        media_type: row.media_type,
        created_at: row.created_at,
    }))
}

/// Appends to the log.
pub async fn record_event<'e, E: PgExecutor<'e>>(
    ex: E,
    kind: EventKind,
    code_id: Option<&CodeId>,
    binding_id: Option<Uuid>,
    detail: &Json,
) -> Result<()> {
    sqlx::query!(
        "INSERT INTO events (kind, code_id, binding_id, detail) VALUES ($1, $2, $3, $4)",
        kind.as_str(),
        code_id.map(CodeId::as_str),
        binding_id,
        detail,
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// How many times a thing has happened to a code. The scan counter.
pub async fn count_events<'e, E: PgExecutor<'e>>(
    ex: E,
    kind: EventKind,
    code_id: &CodeId,
) -> Result<i64> {
    let count = sqlx::query_scalar!(
        "SELECT count(*) FROM events WHERE kind = $1 AND code_id = $2",
        kind.as_str(),
        code_id.as_str(),
    )
    .fetch_one(ex)
    .await?;
    Ok(count.unwrap_or(0))
}
