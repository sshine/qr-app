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
    CodeReleased,
    CodeRevoked,
    AuthFailed,
    PhotoAdded,
    PhotoDeleted,
    PhotoRestored,
    ForwardFollowed,
    ForwardRepointed,
    BlobCollected,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SheetRendered => "sheet.rendered",
            Self::CodeMinted => "code.minted",
            Self::CodeFirstSeen => "code.first_seen",
            Self::CodeBound => "code.bound",
            Self::CodeReleased => "code.released",
            Self::CodeRevoked => "code.revoked",
            Self::AuthFailed => "auth.failed",
            Self::PhotoAdded => "photo.added",
            Self::PhotoDeleted => "photo.deleted",
            Self::PhotoRestored => "photo.restored",
            Self::ForwardFollowed => "forward.followed",
            Self::ForwardRepointed => "forward.repointed",
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
            EventKind::CodeReleased,
            EventKind::CodeRevoked,
            EventKind::AuthFailed,
            EventKind::PhotoAdded,
            EventKind::PhotoDeleted,
            EventKind::PhotoRestored,
            EventKind::ForwardFollowed,
            EventKind::ForwardRepointed,
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
