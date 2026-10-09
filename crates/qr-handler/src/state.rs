//! What every route is handed.

use std::sync::Arc;

use qr_core::{BaseUrl, BlobStore, Store};

use crate::registry::Registry;

/// Limits a handler has to respect, in one place so two handlers cannot disagree.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_upload_bytes: usize,
    /// The guard that matters: a small file can declare enormous dimensions and ask for
    /// gigabytes before anything is decoded.
    pub max_upload_pixels: u32,
    pub strip_original_exif: bool,
}

/// How the capability cookie is written.
#[derive(Debug, Clone, Copy)]
pub struct CookieConfig {
    pub max_age_days: i64,
    pub secure: bool,
}

/// What a handler is given: the database, the blobs and the rules.
///
/// Deliberately not the whole `AppState`. A handler has no business reaching the
/// registry, and not handing it over is cheaper than saying so.
#[derive(Clone)]
pub struct HandlerCtx {
    store: Store,
    blobs: Arc<dyn BlobStore>,
    base: BaseUrl,
    limits: Limits,
}

impl HandlerCtx {
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.blobs
    }

    pub fn base(&self) -> &BaseUrl {
        &self.base
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }
}

struct Inner {
    store: Store,
    blobs: Arc<dyn BlobStore>,
    registry: Registry,
    base: BaseUrl,
    cookies: CookieConfig,
    limits: Limits,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn new(
        store: Store,
        blobs: Arc<dyn BlobStore>,
        registry: Registry,
        base: BaseUrl,
        cookies: CookieConfig,
        limits: Limits,
    ) -> Self {
        Self(Arc::new(Inner {
            store,
            blobs,
            registry,
            base,
            cookies,
            limits,
        }))
    }

    pub fn store(&self) -> &Store {
        &self.0.store
    }

    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.0.blobs
    }

    pub fn registry(&self) -> &Registry {
        &self.0.registry
    }

    pub fn base(&self) -> &BaseUrl {
        &self.0.base
    }

    pub fn cookies(&self) -> CookieConfig {
        self.0.cookies
    }

    pub fn limits(&self) -> Limits {
        self.0.limits
    }

    /// The narrower view a handler gets.
    pub fn handler_ctx(&self) -> HandlerCtx {
        HandlerCtx {
            store: self.0.store.clone(),
            blobs: Arc::clone(&self.0.blobs),
            base: self.0.base.clone(),
            limits: self.0.limits,
        }
    }
}
