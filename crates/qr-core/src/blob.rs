//! Where the bytes live.
//!
//! Content-addressed: the key is the BLAKE3 hash of the content, so writing a blob that
//! is already there is a no-op and an interrupted upload can simply be retried. It also
//! makes the serving side's caching correct by construction, because the bytes under a
//! URL cannot change.

use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWriteExt as _};

use crate::{Error, Result};

/// A content address: 32 bytes, rendered as 64 lowercase hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobKey([u8; 32]);

impl BlobKey {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn parse(hex: &str) -> Result<Self> {
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!("{hex} is not a blob key")));
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
            let hi = hex_value(pair[0])?;
            let lo = hex_value(pair[1])?;
            *slot = (hi << 4) | lo;
        }
        Ok(Self(out))
    }

    pub fn as_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
        }
        out
    }

    /// Two levels of 256, from the first two bytes.
    ///
    /// A flat directory holds a million files on ext4 and XFS, but a PVC's backing
    /// filesystem is not something this code gets to choose, and `ls` on the mount is
    /// the first thing anyone does when a photo goes missing.
    fn shard(&self) -> (String, String) {
        (format!("{:02x}", self.0[0]), format!("{:02x}", self.0[1]))
    }
}

fn hex_value(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(Error::Invalid("a blob key is lowercase hex".to_owned())),
    }
}

fn hex_digit(nibble: u8) -> char {
    char::from(if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    })
}

impl std::fmt::Display for BlobKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_hex())
    }
}

/// An open blob, ready to stream to a response body.
pub struct BlobRead {
    pub byte_size: u64,
    pub reader: Pin<Box<dyn AsyncRead + Send + 'static>>,
}

#[async_trait]
pub trait BlobStore: Send + Sync + 'static {
    /// Writes the bytes under their own hash. Writing one that is already there is a
    /// no-op, which is what makes retrying an interrupted upload safe.
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> Result<()>;

    async fn open(&self, key: &BlobKey) -> Result<BlobRead>;

    async fn exists(&self, key: &BlobKey) -> Result<bool>;

    /// Only the collector calls this, and only for a key no row refers to.
    async fn delete(&self, key: &BlobKey) -> Result<()>;

    /// Can this store still be written to? A full volume is what the readiness probe is
    /// looking for, and it would otherwise show up as mysterious upload failures.
    async fn probe(&self) -> Result<()>;
}

/// Blobs on a mounted filesystem.
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    /// Creates the directories this store needs, and checks it can write to them.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for dir in [root.join("blobs"), root.join("tmp")] {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|source| Error::Blob {
                    action: "create a directory for",
                    path: dir,
                    source,
                })?;
        }
        let store = Self { root };
        store.probe().await?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, key: &BlobKey) -> PathBuf {
        let (a, b) = key.shard();
        self.root.join("blobs").join(a).join(b).join(key.as_hex())
    }

    fn blob_error<'p>(
        action: &'static str,
        path: &'p Path,
    ) -> impl FnOnce(io::Error) -> Error + 'p {
        move |source| Error::Blob {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> Result<()> {
        let final_path = self.path(key);
        if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
            return Ok(());
        }

        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(Self::blob_error("create a directory for", parent))?;
        }

        // Written under a name nothing reads, synced, and only then renamed. A crash
        // halfway otherwise leaves a truncated file at an address that claims to be the
        // hash of its contents, which no later reader could detect.
        let staging = self.root.join("tmp").join(uuid::Uuid::now_v7().to_string());
        let mut file = tokio::fs::File::create(&staging)
            .await
            .map_err(Self::blob_error("create", &staging))?;
        file.write_all(&bytes)
            .await
            .map_err(Self::blob_error("write", &staging))?;
        file.sync_all()
            .await
            .map_err(Self::blob_error("flush", &staging))?;
        drop(file);

        tokio::fs::rename(&staging, &final_path)
            .await
            .map_err(Self::blob_error("move into place", &final_path))?;
        Ok(())
    }

    async fn open(&self, key: &BlobKey) -> Result<BlobRead> {
        let path = self.path(key);
        let file = tokio::fs::File::open(&path).await.map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                Error::NotFound("blob")
            } else {
                Error::Blob {
                    action: "open",
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let byte_size = file
            .metadata()
            .await
            .map_err(Self::blob_error("measure", &path))?
            .len();

        Ok(BlobRead {
            byte_size,
            reader: Box::pin(file),
        })
    }

    async fn exists(&self, key: &BlobKey) -> Result<bool> {
        let path = self.path(key);
        tokio::fs::try_exists(&path)
            .await
            .map_err(Self::blob_error("look for", &path))
    }

    async fn delete(&self, key: &BlobKey) -> Result<()> {
        let path = self.path(key);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Blob {
                action: "delete",
                path,
                source,
            }),
        }
    }

    async fn probe(&self) -> Result<()> {
        let path = self
            .root
            .join("tmp")
            .join(format!("probe-{}", uuid::Uuid::now_v7()));
        tokio::fs::write(&path, b"probe")
            .await
            .map_err(Self::blob_error("write a probe into", &path))?;
        tokio::fs::remove_file(&path)
            .await
            .map_err(Self::blob_error("clean up a probe from", &path))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn a_key_round_trips_through_hex() {
        let key = BlobKey::of(b"what is in the box");
        let again = BlobKey::parse(&key.as_hex()).expect("parse");
        assert_eq!(key, again);
        assert_eq!(key.as_hex().len(), 64);
    }

    #[test]
    fn the_same_bytes_give_the_same_key() {
        assert_eq!(BlobKey::of(b"a"), BlobKey::of(b"a"));
        assert_ne!(BlobKey::of(b"a"), BlobKey::of(b"b"));
    }

    #[test]
    fn anything_that_is_not_sixty_four_hex_digits_is_refused() {
        for bad in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            assert!(BlobKey::parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn a_key_cannot_escape_the_blob_directory() {
        // The key is 32 bytes rendered as hex, never a caller's string, so there is no
        // path for `..` to arrive through. This states that rather than relying on it.
        assert!(BlobKey::parse("../../etc/passwd").is_err());
        let key = BlobKey::of(b"x");
        assert!(!key.as_hex().contains(['.', '/']));
    }

    #[test]
    fn sharding_takes_two_levels_from_the_front_of_the_hash() {
        let key = BlobKey::of(b"what is in the box");
        let (a, b) = key.shard();
        let hex = key.as_hex();
        assert_eq!(a, &hex[0..2]);
        assert_eq!(b, &hex[2..4]);
    }

    #[tokio::test]
    async fn a_blob_written_can_be_read_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FsBlobStore::open(dir.path()).await.expect("open");

        let bytes = Bytes::from_static(b"two winter coats and a tent");
        let key = BlobKey::of(&bytes);
        store.put(&key, bytes.clone()).await.expect("put");

        assert!(store.exists(&key).await.expect("exists"));
        let read = store.open(&key).await.expect("open");
        assert_eq!(read.byte_size, bytes.len() as u64);
    }

    #[tokio::test]
    async fn writing_the_same_blob_twice_is_a_no_op() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FsBlobStore::open(dir.path()).await.expect("open");

        let bytes = Bytes::from_static(b"a bicycle pump");
        let key = BlobKey::of(&bytes);
        store.put(&key, bytes.clone()).await.expect("first");
        store.put(&key, bytes).await.expect("second");
        assert!(store.exists(&key).await.expect("exists"));
    }

    #[tokio::test]
    async fn an_interrupted_write_leaves_nothing_at_the_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FsBlobStore::open(dir.path()).await.expect("open");
        let key = BlobKey::of(b"never written");
        assert!(!store.exists(&key).await.expect("exists"));
        match store.open(&key).await {
            Err(e) => assert!(e.is_not_found(), "wrong error: {e}"),
            Ok(_) => panic!("a blob that was never written should not open"),
        }
    }

    #[tokio::test]
    async fn deleting_a_blob_that_is_not_there_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FsBlobStore::open(dir.path()).await.expect("open");
        store
            .delete(&BlobKey::of(b"absent"))
            .await
            .expect("delete should be idempotent");
    }

    #[tokio::test]
    async fn probing_an_unwritable_root_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FsBlobStore::open(dir.path()).await.expect("open");
        store.probe().await.expect("a fresh store is writable");

        std::fs::remove_dir_all(dir.path().join("tmp")).expect("remove tmp");
        assert!(store.probe().await.is_err());
    }
}
