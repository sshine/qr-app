//! What this crate fails with.

use std::{io, path::PathBuf};

/// The crate's result type, defaulting to its own error.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("the operating system would not supply randomness")]
    Entropy,

    #[error("a {what} of {len} characters is outside the permitted range")]
    BadLength { what: &'static str, len: usize },

    #[error("a {what} contains a character outside the Crockford alphabet")]
    BadCharacter { what: &'static str },

    #[error("a {what} does not pass its check characters")]
    BadChecksum { what: &'static str },

    #[error("{0}")]
    Invalid(String),

    #[error("no such {0}")]
    NotFound(&'static str),

    #[error("that {0} already exists")]
    Conflict(&'static str),

    #[error("the database refused the request")]
    Database(#[source] sqlx::Error),

    #[error("could not {action} the blob at {}", path.display())]
    Blob {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl Error {
    /// A lookup that found nothing, as opposed to one that failed.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::NotFound(_) | Self::Database(sqlx::Error::RowNotFound)
        )
    }

    /// A unique constraint lost a race. Worth retrying with fresh input.
    pub fn is_conflict(&self) -> bool {
        match self {
            Self::Conflict(_) => true,
            Self::Database(e) => e
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation),
            _ => false,
        }
    }

    /// The caller supplied something this crate will never accept, so retrying the same
    /// request cannot help.
    pub fn is_invalid(&self) -> bool {
        matches!(
            self,
            Self::BadLength { .. }
                | Self::BadCharacter { .. }
                | Self::BadChecksum { .. }
                | Self::Invalid(_)
        )
    }
}

impl From<sqlx::Error> for Error {
    fn from(source: sqlx::Error) -> Self {
        Self::Database(source)
    }
}
