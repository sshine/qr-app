//! A sticker that stands in for another one.
//!
//! A box might have the same code printed on two sides. Rather than minting two codes
//! that both need claiming, one of them can carry the other's code and secret: scanning
//! the link proves the link, and the landing route trades that for the target's cookie
//! and redirects. From then on the visitor sees exactly what the first sticker opens,
//! and the service knows nothing about who they are.

use async_trait::async_trait;
use axum::Router;
use maud::{Markup, html};
use qr_core::{Binding, CodeId, Secret, store};
use qr_handler::{
    AppState, ChooserCard, Delegate, Handler, HandlerCtx, HandlerError, HandlerId, HandlerPage,
};
use serde::Deserialize;
use serde_json::json;

/// The icon the chooser shows. Two linked rings, because "link" already means a URL here.
const ICON: &str = r##"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><path d="M10 6.5h4a3.5 3.5 0 0 1 0 7h-1"/><path d="M14 17.5h-4a3.5 3.5 0 0 1 0-7h1"/></svg>"##;

pub struct LinkHandler;

/// What the chooser form posts and what the binding keeps until `on_bind` moves it.
#[derive(Deserialize)]
struct LinkConfig {
    code: String,
    secret: String,
}

impl LinkConfig {
    fn parse(self) -> Result<(CodeId, Secret), HandlerError> {
        let code = CodeId::parse_checksummed(&self.code)
            .map_err(|e| HandlerError::Rejected(format!("that is not a code: {e}")))?;
        let secret = Secret::parse(&self.secret)
            .map_err(|e| HandlerError::Rejected(format!("that is not a secret: {e}")))?;
        Ok((code, secret))
    }
}

#[async_trait]
impl Handler for LinkHandler {
    fn id(&self) -> HandlerId {
        HandlerId::from_static("qr-link")
    }

    fn name(&self) -> &'static str {
        "Link to other QR code"
    }

    fn chooser(&self) -> ChooserCard {
        ChooserCard {
            title: "Link to other QR code",
            blurb: "This sticker opens whatever the other sticker opens. Point the camera at the other one.",
            icon: maud::PreEscaped(ICON),
            fields: fields(),
            submit: false,
        }
    }

    fn config_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["code", "secret"],
            "properties": {
                "code": { "type": "string" },
                "secret": { "type": "string" },
            },
        })
    }

    /// Proves both halves before a binding exists.
    ///
    /// The secret is kept in the returned config so `on_bind` can write it to the link
    /// row inside the claim's transaction; `on_bind` strips it from the binding again.
    async fn validate_config(
        &self,
        ctx: &HandlerCtx,
        config: serde_json::Value,
    ) -> Result<serde_json::Value, HandlerError> {
        let link: LinkConfig = serde_json::from_value(config)
            .map_err(|e| HandlerError::Rejected(format!("that is not a link: {e}")))?;
        let (target, secret) = link.parse()?;
        let mut conn = ctx
            .store()
            .pool()
            .acquire()
            .await
            .map_err(qr_core::Error::from)?;
        prove(&mut conn, &target, &secret).await?;
        Ok(json!({ "code": target.as_str(), "secret": secret.expose() }))
    }

    async fn on_bind(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        binding: &Binding,
    ) -> Result<(), HandlerError> {
        let config: LinkConfig = serde_json::from_value(binding.config.clone())
            .map_err(|e| HandlerError::Rejected(format!("that is not a link: {e}")))?;
        let (target, secret) = config.parse()?;

        if target == binding.code_id {
            return Err(HandlerError::Rejected(
                "a sticker cannot link to itself".to_owned(),
            ));
        }

        // Checked again rather than trusting `validate_config`: the intended-handler
        // path from a printed sheet calls `on_bind` without it.
        prove(tx, &target, &secret).await?;

        store::insert_link(&mut **tx, binding.id, &target, &secret).await?;
        // The secret lives in code_links, never in bindings.config, which `describe`
        // and the API can serialise.
        store::set_binding_config(&mut **tx, binding.id, &json!({ "code": target.as_str() }))
            .await?;
        Ok(())
    }

    async fn describe(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<serde_json::Value, HandlerError> {
        let stored = store::link(ctx.store().pool(), binding.id).await?;
        Ok(match stored {
            Some(link) => json!({
                "code": link.target.as_str(),
                "url": format!("/c/{}", link.target),
                "linked": true,
            }),
            None => json!({ "linked": false }),
        })
    }

    async fn render(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<HandlerPage, HandlerError> {
        let stored = store::link(ctx.store().pool(), binding.id).await?;
        Ok(HandlerPage::Markup(html! {
            main class="link" {
                h1 { "Link to other QR code" }
                p class="code-id" { (binding.code_id.grouped()) }
                @match &stored {
                    Some(link) => {
                        p { "Scanning this sticker opens" }
                        p class="destination" {
                            a href={ "/c/" (link.target) } { (link.target.grouped()) }
                        }
                        p class="hint" {
                            "Scanning it again hands the other code's cookie over, so no
                             secret has to be typed twice."
                        }
                    }
                    None => p class="destination empty" {
                        "This link has no destination. Release it and link it again."
                    },
                }
            }
        }))
    }

    async fn delegate(
        &self,
        ctx: &HandlerCtx,
        binding: &Binding,
    ) -> Result<Option<Delegate>, HandlerError> {
        Ok(store::link(ctx.store().pool(), binding.id)
            .await?
            .map(|link| Delegate {
                code: link.target,
                secret: link.secret,
            }))
    }

    fn pages(&self) -> Router<AppState> {
        Router::new()
    }

    fn routes(&self) -> Router<AppState> {
        Router::new()
    }
}

/// The camera the link card carries.
///
/// No typed code and no file picker: the proof has to come from the other sticker's own
/// symbol. The hidden fields are filled only after a decode, and the confirmation button
/// submits the same claim form a chooser card always posts.
fn fields() -> Markup {
    html! {
        noscript {
            p class="hint" {
                "Linking one sticker to another needs JavaScript and a camera. Open this
                 page in a browser that can use one."
            }
        }
        div class="link-scan" x-data="linkScan" x-cloak {
            button type="button" class="scan-start" x-show="showStart"
                x-bind:disabled="starting" x-on:click="start" {
                "Scan the other sticker"
            }
            p class="scan-status" x-show="starting" { "Asking for the camera…" }
            div class="viewfinder" x-show="showCamera" x-cloak {
                video x-ref="video" playsinline muted autoplay aria-label="Camera preview" {}
                div class="viewfinder-frame" aria-hidden="true" {}
            }
            p class="scan-status" x-show="showCamera" {
                "Point the camera at the QR code on the other sticker."
            }
            div class="scan-actions" x-show="showCamera" x-cloak {
                button type="button" class="scan-again" x-on:click="reset" {
                    "Stop camera"
                }
            }
            p class="error" role="alert" x-show="hasError" x-text="error" {}
            div class="scan-result" x-show="showResult" x-cloak aria-live="polite" {
                p class="scan-found" {
                    "Found sticker "
                    span class="code-id" x-text="codeDisplay" {}
                }
                p class="hint" { "It opens the same thing as this one." }
                div class="scan-actions" {
                    button type="submit" class="card-claim" { "Use this sticker" }
                    button type="button" class="scan-again" x-on:click="reset" {
                        "Scan another"
                    }
                }
                input type="hidden" name="config.code" x-bind:value="code";
                input type="hidden" name="config.secret" x-bind:value="secret";
            }
        }
    }
}

/// That the target can be opened at all, and that it is already used.
///
/// A link to an unclaimed code would open the chooser on the other side, which is not
/// the same thing as the two stickers being one box. It is also unrecoverable if the
/// target is never used, so it is refused while the visitor can still pick another.
async fn prove(
    conn: &mut sqlx::PgConnection,
    target: &CodeId,
    secret: &Secret,
) -> Result<(), HandlerError> {
    let target = store::redeem(&mut *conn, target, secret)
        .await?
        .ok_or_else(|| {
            HandlerError::Rejected("that code and secret do not open anything".to_owned())
        })?;

    if store::active_binding(&mut *conn, &target.id)
        .await?
        .is_none()
    {
        return Err(HandlerError::Rejected(
            "that code has not been used yet, so there is nothing to link to".to_owned(),
        ));
    }

    Ok(())
}
