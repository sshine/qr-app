//! The domain under qr.shine.town: what a code is, what proves you may open it, and where
//! the bytes behind it live.
//!
//! This crate knows nothing about HTTP. Everything here is equally usable from the server,
//! from the sheet renderer and from a test, which is what keeps the capability rules in
//! one place rather than spread across request handlers.

pub mod blob;
pub mod error;
pub mod ids;
pub mod model;
pub mod store;

#[cfg(feature = "testing")]
pub mod testing;
pub mod url;

pub use blob::{BlobKey, BlobRead, BlobStore, FsBlobStore};
pub use error::{Error, Result};
pub use ids::{CodeId, Secret, SecretHash, decoy_hash};
pub use model::{
    Binding, BlobRecord, Code, CodeLink, EventKind, Location, MintedCode, NewLocation, Sheet,
};
pub use store::Store;
pub use url::BaseUrl;
