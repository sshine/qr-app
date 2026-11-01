//! The rows, as Rust sees them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CodeId, Secret, SecretHash};

/// One printing run.
#[derive(Debug, Clone)]
pub struct Sheet {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub code_count: i32,
    pub base_url: String,
    pub preset: String,
    pub layout: serde_json::Value,
    /// What every code on this sheet is destined to become, or `None` for the chooser.
    pub handler_id: Option<String>,
    pub handler_config: serde_json::Value,
    pub note: Option<String>,
    pub pdf_key: Option<String>,
    pub pdf_expires_at: Option<DateTime<Utc>>,
}

/// One sticker.
#[derive(Debug, Clone)]
pub struct Code {
    pub id: CodeId,
    pub sheet_id: Option<Uuid>,
    pub secret_hash: SecretHash,
    pub intended_handler: Option<String>,
    pub intended_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Hidden from the overview. The sticker still scans and still opens what it holds.
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Code {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// A freshly minted code, the one moment its secret exists in plaintext.
#[derive(Debug, Clone)]
pub struct MintedCode {
    pub id: CodeId,
    pub secret: Secret,
}

/// What a code became.
#[derive(Debug, Clone)]
pub struct Binding {
    pub id: Uuid,
    pub code_id: CodeId,
    pub handler_id: String,
    pub config: serde_json::Value,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

/// A sticker that stands in for another one.
///
/// The secret is kept so a scan of the link can hand the visitor the target's cookie
/// without asking for it again. It is deliberately not `Serialize`: nothing outside the
/// store should be able to turn it into JSON by deriving a trait.
#[derive(Debug, Clone)]
pub struct CodeLink {
    pub target: CodeId,
    pub secret: Secret,
}

/// A place a sticker can be said to be.
///
/// Shared between codes: the point of a catalog is that the second box in the loft is
/// tagged by picking the loft, not by typing it again on a phone.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Location {
    pub id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub created_at: DateTime<Utc>,
}

/// A place as it is created, before it has an id.
#[derive(Debug, Clone)]
pub struct NewLocation {
    pub name: String,
    pub address: Option<String>,
    pub lat: f64,
    pub lon: f64,
}

/// A name a sticker can carry.
///
/// Shared between codes for the same reason [`Location`] is: picking an existing tag is
/// faster and less error-prone than retyping a near-spelling of it on a phone.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// A stored blob's metadata.
#[derive(Debug, Clone)]
pub struct BlobRecord {
    pub key: String,
    pub byte_size: i64,
    pub media_type: String,
    pub created_at: DateTime<Utc>,
}

/// What a code is destined to become when it is first scanned.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Intent {
    pub handler: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// The kinds `events.kind` holds.
///
/// A closed set rather than free text, so a query over the log cannot silently miss a
/// spelling nobody else used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventKind {
    SheetRendered,
    CodeMinted,
    CodeFirstSeen,
    CodeBound,
    CodeRetitled,
    CodeReleased,
    CodeRecovered,
    CodeRevoked,
    CodeDeleted,
    CodeRestored,
    AuthFailed,
    PhotoAdded,
    PhotoDeleted,
    PhotoRestored,
    ForwardFollowed,
    ForwardRepointed,
    LocationTagged,
    TagAdded,
    TagRemoved,
    BlobCollected,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SheetRendered => "sheet.rendered",
            Self::CodeMinted => "code.minted",
            Self::CodeFirstSeen => "code.first_seen",
            Self::CodeBound => "code.bound",
            Self::CodeRetitled => "code.retitled",
            Self::CodeReleased => "code.released",
            Self::CodeRecovered => "code.recovered",
            Self::CodeRevoked => "code.revoked",
            Self::CodeDeleted => "code.deleted",
            Self::CodeRestored => "code.restored",
            Self::AuthFailed => "auth.failed",
            Self::PhotoAdded => "photo.added",
            Self::PhotoDeleted => "photo.deleted",
            Self::PhotoRestored => "photo.restored",
            Self::ForwardFollowed => "forward.followed",
            Self::ForwardRepointed => "forward.repointed",
            Self::LocationTagged => "location.tagged",
            Self::TagAdded => "tag.added",
            Self::TagRemoved => "tag.removed",
            Self::BlobCollected => "blob.collected",
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_kind_is_a_dotted_pair() {
        for kind in [
            EventKind::SheetRendered,
            EventKind::CodeMinted,
            EventKind::CodeFirstSeen,
            EventKind::CodeBound,
            EventKind::CodeRetitled,
            EventKind::CodeReleased,
            EventKind::CodeRecovered,
            EventKind::CodeRevoked,
            EventKind::CodeDeleted,
            EventKind::CodeRestored,
            EventKind::AuthFailed,
            EventKind::PhotoAdded,
            EventKind::PhotoDeleted,
            EventKind::PhotoRestored,
            EventKind::ForwardFollowed,
            EventKind::ForwardRepointed,
            EventKind::LocationTagged,
            EventKind::TagAdded,
            EventKind::TagRemoved,
            EventKind::BlobCollected,
        ] {
            let s = kind.as_str();
            assert_eq!(s.split('.').count(), 2, "{s} is not subject.verb");
            assert!(
                s.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
            );
        }
    }
}
