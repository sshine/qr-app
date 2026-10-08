//! Where a forwarding code points, and where it pointed before.

use chrono::{DateTime, Utc};
use qr_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use uuid::Uuid;

/// A destination, as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardTarget {
    pub id: Uuid,
    pub url: String,
    pub status: i16,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub superseded_at: Option<DateTime<Utc>>,
}

/// A destination, as submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTarget {
    pub url: String,
    #[serde(default = "default_status")]
    pub status: i16,
    #[serde(default)]
    pub note: Option<String>,
}

const fn default_status() -> i16 {
    // Temporary by default, because the whole point of these codes is that the
    // destination can change, and a browser that cached a 301 would never ask again.
    302
}

impl NewTarget {
    pub fn validate(&self) -> std::result::Result<(), String> {
        let parsed =
            url::Url::parse(&self.url).map_err(|e| format!("{} is not a URL: {e}", self.url))?;

        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("{} must be http or https", self.url));
        }
        if parsed.host().is_none() {
            return Err(format!("{} names no host", self.url));
        }
        if !matches!(self.status, 301 | 302 | 307 | 308) {
            return Err(format!("{} is not a redirect status", self.status));
        }
        if self.note.as_ref().is_some_and(|n| n.chars().count() > 280) {
            return Err("that note is too long".to_owned());
        }

        // A destination under /c/ would send a scanner back into this service, and a
        // code pointing at itself is an infinite redirect a browser gives up on rather
        // than reports.
        if parsed.path().starts_with("/c/") {
            return Err("a destination inside this service's own code paths would loop".to_owned());
        }

        Ok(())
    }
}

/// The destination in force, if there is one.
pub async fn current<'e, E: PgExecutor<'e>>(
    ex: E,
    binding_id: Uuid,
) -> Result<Option<ForwardTarget>> {
    let row = sqlx::query!(
        r#"
        SELECT id, url, status, note, created_at, created_by, superseded_at
        FROM forward_targets
        WHERE binding_id = $1 AND superseded_at IS NULL
        "#,
        binding_id,
    )
    .fetch_optional(ex)
    .await?;

    Ok(row.map(|row| ForwardTarget {
        id: row.id,
        url: row.url,
        status: row.status,
        note: row.note,
        created_at: row.created_at,
        created_by: row.created_by,
        superseded_at: row.superseded_at,
    }))
}

/// Everything this code has ever pointed at, newest first.
pub async fn history<'e, E: PgExecutor<'e>>(ex: E, binding_id: Uuid) -> Result<Vec<ForwardTarget>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, url, status, note, created_at, created_by, superseded_at
        FROM forward_targets
        WHERE binding_id = $1
        ORDER BY created_at DESC
        "#,
        binding_id,
    )
    .fetch_all(ex)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ForwardTarget {
            id: row.id,
            url: row.url,
            status: row.status,
            note: row.note,
            created_at: row.created_at,
            created_by: row.created_by,
            superseded_at: row.superseded_at,
        })
        .collect())
}

/// Points the code somewhere new.
///
/// Supersedes rather than overwrites, so a poster whose destination moved can be traced
/// back to what it used to say. Both statements run on the caller's executor, which is
/// a transaction, so there is never a moment with two live rows or none.
pub async fn set(
    conn: &mut sqlx::PgConnection,
    binding_id: Uuid,
    new: &NewTarget,
    by: Option<&str>,
) -> Result<ForwardTarget> {
    new.validate().map_err(Error::Invalid)?;

    sqlx::query!(
        r#"
        UPDATE forward_targets SET superseded_at = now()
        WHERE binding_id = $1 AND superseded_at IS NULL
        "#,
        binding_id,
    )
    .execute(&mut *conn)
    .await?;

    let row = sqlx::query!(
        r#"
        INSERT INTO forward_targets (id, binding_id, url, status, note, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, url, status, note, created_at, created_by, superseded_at
        "#,
        Uuid::now_v7(),
        binding_id,
        new.url,
        new.status,
        new.note,
        by,
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(ForwardTarget {
        id: row.id,
        url: row.url,
        status: row.status,
        note: row.note,
        created_at: row.created_at,
        created_by: row.created_by,
        superseded_at: row.superseded_at,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn target(url: &str) -> NewTarget {
        NewTarget {
            url: url.to_owned(),
            status: 302,
            note: None,
        }
    }

    #[test]
    fn an_ordinary_destination_is_accepted() {
        assert!(target("https://example.com/poster").validate().is_ok());
        assert!(target("http://example.com").validate().is_ok());
    }

    #[test]
    fn a_destination_that_is_not_http_is_refused() {
        for bad in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(target(bad).validate().is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_destination_back_into_a_code_path_is_refused() {
        let err = target("https://qr.shine.town/c/ABCD123456")
            .validate()
            .expect_err("should loop");
        assert!(err.contains("loop"), "{err}");
    }

    #[test]
    fn only_a_redirect_status_is_accepted() {
        for status in [200i16, 404, 500, 303] {
            let mut t = target("https://example.com");
            t.status = status;
            assert!(t.validate().is_err(), "{status} should be refused");
        }
        for status in [301i16, 302, 307, 308] {
            let mut t = target("https://example.com");
            t.status = status;
            assert!(t.validate().is_ok(), "{status} should be accepted");
        }
    }

    #[test]
    fn the_default_is_temporary() {
        // A browser that cached a permanent redirect would never ask again, and asking
        // again is the entire point of these codes.
        let parsed: NewTarget =
            serde_json::from_str(r#"{"url":"https://example.com"}"#).expect("parse");
        assert_eq!(parsed.status, 302);
    }
}
