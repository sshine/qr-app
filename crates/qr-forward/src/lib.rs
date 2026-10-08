//! A code that redirects, and can be repointed later.
//!
//! The simpler of the two handlers, and the one that exercises the parts of the trait a
//! photo album does not: a response that is not markup, a viewer distinction, and a
//! configuration that matters before anything has been uploaded.
//!
//! Anyone holding the sticker follows the redirect. Only an admin sees the editor, and
//! the editor is at the same URL — which is the whole reason `render` is handed a
//! viewer rather than just a binding.

mod target;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use maud::{Markup, html};
use qr_core::{Binding, model::EventKind, store};
use qr_handler::{
    AppState, ChooserCard, CodeCtx, Handler, HandlerCtx, HandlerError, HandlerId, HandlerPage,
    RequireAdmin, Viewer,
};
use serde_json::json;

pub use target::{ForwardTarget, NewTarget};

/// The icon the chooser shows. Inline, so the chooser needs no second request.
const ICON: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.5 1.5"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.5-1.5"/></svg>"##;

pub struct ForwardHandler;

#[async_trait]
impl Handler for ForwardHandler {
    fn id(&self) -> HandlerId {
        HandlerId::from_static("forward")
    }

    fn name(&self) -> &'static str {
        "Link"
    }

    fn chooser(&self) -> ChooserCard {
        ChooserCard {
            title: "Link",
            blurb: "Send whoever scans this somewhere, and change where later.",
            icon: maud::PreEscaped(ICON),
        }
    }

    fn config_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "format": "uri" },
                "status": { "type": "integer", "enum": [301, 302, 307, 308] },
                "note": { "type": "string" },
            },
        })
    }

    /// Accepts a destination at claim time, so a code minted through the API with a URL
    /// in its config is live the moment it is first scanned.
    async fn validate_config(
        &self,
        config: serde_json::Value,
    ) -> Result<serde_json::Value, HandlerError> {
        if config.get("url").is_none() {
            return Ok(json!({}));
        }
        let target: NewTarget = serde_json::from_value(config)
            .map_err(|e| HandlerError::Rejected(format!("that is not a destination: {e}")))?;
        target.validate().map_err(HandlerError::Rejected)?;
        serde_json::to_value(&target)
            .map_err(|e| HandlerError::Rejected(format!("that is not a destination: {e}")))
    }

    /// Writes the destination inside the transaction that creates the binding, so a
    /// code minted with one is never briefly bound with nowhere to go.
    async fn on_bind(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        binding: &Binding,
    ) -> Result<(), HandlerError> {
        if binding.config.get("url").is_none() {
            return Ok(());
        }
        let target: NewTarget = serde_json::from_value(binding.config.clone())
            .map_err(|e| HandlerError::Rejected(e.to_string()))?;
        target::set(tx, binding.id, &target, Some("claim")).await?;
        Ok(())
    }

    async fn describe(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
        viewer: &Viewer,
    ) -> Result<serde_json::Value, HandlerError> {
        let current = target::current(ctx.store().pool(), binding.id).await?;
        let follows = store::count_events(
            ctx.store().pool(),
            EventKind::ForwardFollowed,
            &binding.code_id,
        )
        .await?;

        let mut described = json!({
            "url": current.as_ref().map(|t| t.url.clone()),
            "status": current.as_ref().map(|t| t.status),
            "note": current.as_ref().and_then(|t| t.note.clone()),
            "follows": follows,
        });

        // History is only ever shown to an admin, so it is only ever fetched for one.
        if viewer.is_admin() {
            let history = target::history(ctx.store().pool(), binding.id).await?;
            described["history"] = serde_json::to_value(&history).unwrap_or(json!([]));
        }

        Ok(described)
    }

    async fn render(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
        viewer: &Viewer,
    ) -> Result<HandlerPage, HandlerError> {
        let current = target::current(ctx.store().pool(), binding.id).await?;

        // An admin gets the editor at the same URL an anonymous visitor is redirected
        // from. Without this there would be no way back to a code that works.
        if viewer.is_admin() {
            let described = self.describe(ctx, binding, viewer).await?;
            return Ok(HandlerPage::Markup(editor(
                binding,
                current.as_ref(),
                &described,
            )));
        }

        let Some(target) = current else {
            return Ok(HandlerPage::Markup(unset()));
        };

        // Counted before the redirect is sent, because nothing comes back afterwards to
        // count it. A failure here must not swallow the redirect.
        if let Err(error) = store::record_event(
            ctx.store().pool(),
            EventKind::ForwardFollowed,
            Some(&binding.code_id),
            Some(binding.id),
            &json!({}),
        )
        .await
        {
            tracing::warn!(%error, "a follow was not counted");
        }

        let status = StatusCode::from_u16(target.status.try_into().unwrap_or(302))
            .unwrap_or(StatusCode::FOUND);

        Ok(HandlerPage::Response(
            (
                status,
                [
                    (header::LOCATION, target.url.clone()),
                    // The destination can change under the same URL, so nothing may
                    // remember this one.
                    (header::CACHE_CONTROL, "no-store".to_owned()),
                ],
            )
                .into_response(),
        ))
    }

    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/target", get(read_target).put(write_target))
            .route("/history", get(read_history))
    }
}

/// Anyone holding the sticker may see where it points.
async fn read_target(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let Some(binding) = access.binding else {
        return (StatusCode::NOT_FOUND, "not claimed\n").into_response();
    };
    match target::current(state.store().pool(), binding.id).await {
        Ok(target) => Json(json!({ "target": target })).into_response(),
        Err(error) => error_response(&error),
    }
}

/// Only an admin may change it.
///
/// A public code is scanned by strangers, and every one of them holds the capability by
/// the time they have seen it. The capability is therefore not what may repoint it.
async fn write_target(
    CodeCtx(access): CodeCtx,
    RequireAdmin(admin): RequireAdmin,
    State(state): State<AppState>,
    Json(new): Json<NewTarget>,
) -> Response {
    let Some(binding) = access.binding else {
        return (StatusCode::NOT_FOUND, "not claimed\n").into_response();
    };
    if let Err(reason) = new.validate() {
        return (StatusCode::BAD_REQUEST, format!("{reason}\n")).into_response();
    }

    let result = async {
        let mut tx = state.store().begin().await?;
        let target = target::set(&mut tx, binding.id, &new, Some(&admin.subject)).await?;
        store::record_event(
            &mut *tx,
            EventKind::ForwardRepointed,
            Some(&access.code.id),
            Some(binding.id),
            &json!({ "url": new.url, "by": admin.subject }),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, qr_core::Error>(target)
    }
    .await;

    match result {
        Ok(target) => Json(json!({ "target": target })).into_response(),
        Err(error) => error_response(&error),
    }
}

async fn read_history(
    CodeCtx(access): CodeCtx,
    RequireAdmin(_): RequireAdmin,
    State(state): State<AppState>,
) -> Response {
    let Some(binding) = access.binding else {
        return (StatusCode::NOT_FOUND, "not claimed\n").into_response();
    };
    match target::history(state.store().pool(), binding.id).await {
        Ok(history) => Json(json!({ "history": history })).into_response(),
        Err(error) => error_response(&error),
    }
}

fn error_response(error: &qr_core::Error) -> Response {
    if error.is_invalid() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "a forwarding request failed");
    (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
}

/// What an anonymous visitor sees before a destination has been set.
fn unset() -> Markup {
    html! {
        main class="forward unset" {
            h1 { "Nothing here yet" }
            p { "This code does not point anywhere. Whoever owns it can set a destination." }
        }
    }
}

/// The editor, shown at the same URL when an admin asks for it.
fn editor(
    binding: &Binding,
    current: Option<&ForwardTarget>,
    described: &serde_json::Value,
) -> Markup {
    let follows = described
        .get("follows")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    html! {
        main class="forward editor" x-data="forward" {
            h1 { "Link" }
            p class="code-id" { (binding.code_id.grouped()) }

            @if let Some(target) = current {
                p class="destination" {
                    "Points at "
                    a href=(target.url) rel="noreferrer noopener" { (target.url) }
                }
                p class="follows" { (follows) " scans" }
            } @else {
                p class="destination empty" { "No destination set." }
            }

            form x-ref="form" "x-on:submit.prevent"="save" class="target-form" {
                label {
                    "Destination"
                    input type="url" name="url" x-model="url" required
                        placeholder="https://example.com" autocomplete="off";
                }
                label {
                    "Note"
                    input type="text" name="note" x-model="note" autocomplete="off";
                }
                button type="submit" "x-bind:disabled"="saving" { "Save" }
                span class="error" x-text="error" x-show="error" {}
            }
        }
    }
}
