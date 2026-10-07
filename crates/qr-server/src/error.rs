//! Why the service refuses to start.
//!
//! Each message says what to set rather than what went wrong, because the reader is
//! looking at a container that will not come up and has only this line to go on.

use std::{io, net::SocketAddr, path::PathBuf};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartupError {
    #[error(
        "no database connection string. Set QR_DATABASE_URL, or mount the database \
         secret's `uri` key and point QR_DATABASE_URL_FILE at it."
    )]
    NoDatabase,

    #[error("could not read the database connection string from {}", path.display())]
    DatabaseUrlFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not reach the database")]
    Database(#[source] sqlx::Error),

    #[error("the migrations did not apply")]
    Migrate(#[source] qr_core::Error),

    #[error("--base-url is not a base this service can print stickers for")]
    BaseUrl(#[source] qr_core::Error),

    #[error("could not open the blob store at {}", path.display())]
    BlobRoot {
        path: PathBuf,
        #[source]
        source: qr_core::Error,
    },

    #[error("could not read the admin token from {}", path.display())]
    AdminToken {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "--insecure-cookies drops the Secure flag from the capability cookie, so it is \
         refused unless --bind is loopback; it is {addr}"
    )]
    InsecureCookiesOffLoopback { addr: SocketAddr },

    #[error("could not listen on {addr}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },

    #[error("the server stopped unexpectedly")]
    Serve(#[source] io::Error),

    #[error(transparent)]
    Sheet(#[from] qr_sheet::Error),

    #[error(transparent)]
    Core(#[from] qr_core::Error),

    #[error("could not write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
