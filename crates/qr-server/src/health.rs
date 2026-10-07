//! The probe listener.
//!
//! A second port rather than a path on the public one: the probes must answer whether or
//! not the public listener's body limits, timeouts and tracing are in the way, /readyz
//! queries the database and is not something to route from the internet, and a metrics
//! scrape has no business in the public access log.

use std::sync::Arc;

use axum::{Json, Router, http::StatusCode, routing::get};
use qr_core::{BlobStore, Store};
use serde_json::json;

#[derive(Clone)]
pub struct Health {
    store: Store,
    blobs: Arc<dyn BlobStore>,
}

impl Health {
    pub fn new(store: Store, blobs: Arc<dyn BlobStore>) -> Self {
        Self { store, blobs }
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/healthz", get(alive))
            .route("/readyz", get(ready))
            .with_state(self)
    }
}

/// Is the process running.
///
/// Touches nothing. A liveness probe that queries the database turns a failover into a
/// crash loop, which is a worse outage than the one it was watching for.
async fn alive() -> &'static str {
    "ok\n"
}

/// Can the process serve.
///
/// Both dependencies, named individually in the body: a full volume and an unreachable
/// database look identical from outside, and the first otherwise shows up as uploads
/// that mysteriously fail.
async fn ready(
    axum::extract::State(health): axum::extract::State<Health>,
) -> (StatusCode, Json<serde_json::Value>) {
    let database = health.store.probe().await.err().map(|e| e.to_string());
    let blobs = health.blobs.probe().await.err().map(|e| e.to_string());

    let status = if database.is_none() && blobs.is_none() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(json!({
            "database": database.clone().map_or_else(|| json!("ok"), serde_json::Value::String),
            "blobs": blobs.clone().map_or_else(|| json!("ok"), serde_json::Value::String),
        })),
    )
}
