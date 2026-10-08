//! What a code can become.

use async_trait::async_trait;
use maud::Markup;
use qr_core::Binding;
use sqlx::{Postgres, Transaction};

use crate::{HandlerCtx, HandlerId, Viewer, state::AppState};

/// How a handler advertises itself on an unbound code.
pub struct ChooserCard {
    pub title: &'static str,
    /// A sentence or two under the title.
    pub blurb: &'static str,
    /// An inline `<svg>`, so the chooser needs no second request and no icon font.
    pub icon: maud::PreEscaped<&'static str>,
}

/// What `render` gives back.
pub enum HandlerPage {
    /// Wrapped in the shared shell, which supplies the doctype, the head, the script
    /// tag and the hydration payload, so a handler cannot forget any of them.
    Markup(Markup),
    /// Bypasses the shell. A redirect, a file, anything with a status of its own.
    Response(axum::response::Response),
}

impl From<Markup> for HandlerPage {
    fn from(markup: Markup) -> Self {
        Self::Markup(markup)
    }
}

/// What the core has to turn into a status code.
///
/// Deliberately coarse. A handler's own domain errors stay inside the handler and are
/// rendered there; only what the core must act on comes back out.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandlerError {
    #[error("storage failed")]
    Store(#[from] qr_core::Error),

    #[error("{0}")]
    Rejected(String),

    #[error("not authorised")]
    Forbidden,

    #[error("nothing here")]
    NotFound,
}

impl HandlerError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound) || matches!(self, Self::Store(e) if e.is_not_found())
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }

    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Forbidden)
    }
}

#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// Stable, and never reused for a different handler.
    fn id(&self) -> HandlerId;

    /// What a human calls it.
    fn name(&self) -> &'static str;

    /// How the chooser advertises it.
    fn chooser(&self) -> ChooserCard;

    /// Extra `<head>` content for pages this handler renders. The stylesheet, the
    /// script and the hydration payload are already there; this is for a handler that
    /// needs something more.
    fn head(&self) -> Markup {
        maud::html! {}
    }

    /// A JSON Schema for this handler's `config`, served at /api/v1/handlers so a
    /// script minting codes can see what it may send.
    fn config_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    /// The config a fresh binding starts with.
    fn default_config(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    /// Checks and normalises a config before it is stored.
    async fn validate_config(
        &self,
        config: serde_json::Value,
    ) -> Result<serde_json::Value, HandlerError> {
        Ok(config)
    }

    /// Called inside the transaction that creates the binding, so a handler needing a
    /// row of its own can write it atomically with the claim.
    async fn on_bind(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        binding: &Binding,
    ) -> Result<(), HandlerError> {
        let _ = (tx, binding);
        Ok(())
    }

    /// Called before a binding is marked released. Handler rows are kept rather than
    /// deleted, keyed by binding, so this is for cleanup a handler actually wants.
    async fn on_release(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        binding: &Binding,
    ) -> Result<(), HandlerError> {
        let _ = (tx, binding);
        Ok(())
    }

    /// The JSON behind this binding: what the API returns and what the page hydrates
    /// from, so the two cannot drift apart.
    async fn describe(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
        viewer: &Viewer,
    ) -> Result<serde_json::Value, HandlerError>;

    /// The body of `GET /c/{code}` for a code bound to this handler.
    async fn render(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
        viewer: &Viewer,
    ) -> Result<HandlerPage, HandlerError>;

    /// Routes mounted under `/c/{code}/api/v1/h/<id>`.
    ///
    /// The capability layer has already run over the whole subtree, so a handler reads
    /// `CodeCtx` and never has to check anything itself.
    fn routes(&self) -> axum::Router<AppState>;
}
