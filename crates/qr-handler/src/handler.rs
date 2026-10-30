//! What a code can become.

use async_trait::async_trait;
use maud::Markup;
use qr_core::Binding;
use sqlx::{Postgres, Transaction};

use crate::{HandlerCtx, HandlerId, state::AppState};

/// How a handler advertises itself on an unbound code.
pub struct ChooserCard {
    pub title: &'static str,
    /// A sentence or two under the title.
    pub blurb: &'static str,
    /// An inline `<svg>`, so the chooser needs no second request and no icon font.
    pub icon: maud::PreEscaped<&'static str>,
    /// Extra controls inside the chooser's form, for a handler that needs a value
    /// before it can be claimed. Names beginning `config.` become that handler's config,
    /// which is how a form post works without any JavaScript.
    pub fields: Markup,
}

/// A target this handler hands the visitor over to.
///
/// The link handler returns one, and the landing route turns it into a redirect that also
/// sets the target code's capability cookie. Carrying the secret here rather than in the
/// handler's `render` keeps cookie writing in one place.
pub struct Delegate {
    pub code: qr_core::CodeId,
    pub secret: qr_core::Secret,
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
    ///
    /// Given the handler context because a handler may have to look something up
    /// before it can accept a config — the link handler proves the target's secret this
    /// way. Runs before the binding exists, so this is for checks that need no row of
    /// their own.
    async fn validate_config(
        &self,
        ctx: &HandlerCtx,
        config: serde_json::Value,
    ) -> Result<serde_json::Value, HandlerError> {
        let _ = ctx;
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
    ) -> Result<serde_json::Value, HandlerError>;

    /// The body of `GET /c/{code}` for a code bound to this handler.
    async fn render(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<HandlerPage, HandlerError>;

    /// The body of `GET /c/{code}/manage`.
    ///
    /// The page a handler shows when the code is being administered rather than used.
    /// Defaults to the ordinary page, which is right for an album; a handler that sends
    /// the visitor elsewhere overrides it so the code can still be released.
    async fn manage(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<HandlerPage, HandlerError> {
        self.render(ctx, binding).await
    }

    /// Where a scan of this binding should really go, if anywhere.
    ///
    /// The landing route follows it, setting the target's capability cookie as it
    /// redirects, so a sticker on the other side of a box opens exactly what the first
    /// one does.
    async fn delegate(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<Option<Delegate>, HandlerError> {
        let _ = (ctx, binding);
        Ok(None)
    }

    /// Pages this handler owns, mounted under `/c/{code}/h/{id}`.
    ///
    /// Not the API: what lives here is HTML a visitor can navigate to, such as a
    /// full-width view of one photo. The capability layer has already run.
    fn pages(&self) -> axum::Router<AppState> {
        axum::Router::new()
    }

    /// Routes mounted under `/c/{code}/api/v1/h/<id>`.
    ///
    /// The capability layer has already run over the whole subtree, so a handler reads
    /// `CodeCtx` and never has to check anything itself.
    fn routes(&self) -> axum::Router<AppState>;
}
