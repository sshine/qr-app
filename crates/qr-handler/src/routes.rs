//! The routes every code has, whatever it became.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use maud::html;
use qr_core::{Binding, CodeId, Secret, model::EventKind, store};
use serde::Deserialize;
use serde_json::json;

use crate::{
    HandlerPage,
    capability::{self, CodeCtx},
    shell::{self, Page},
    state::AppState,
};

/// Everything a handler failure turns into.
fn handler_response(error: &crate::HandlerError) -> Response {
    if error.is_not_found() {
        return (StatusCode::NOT_FOUND, "nothing here\n").into_response();
    }
    if error.is_forbidden() {
        return (StatusCode::FORBIDDEN, "not authorised\n").into_response();
    }
    if error.is_rejected() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "a handler failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong\n").into_response()
}

fn store_response(error: &qr_core::Error) -> Response {
    if error.is_conflict() {
        return (StatusCode::CONFLICT, "that already happened\n").into_response();
    }
    if error.is_invalid() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "a request could not reach the database");
    (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
}

/// `GET /c/{code}` — the page a scan lands on.
///
/// Unguarded on purpose: this is the one route that has to answer before a cookie
/// exists, because the secret is still in the fragment and only a browser can see it.
pub async fn landing(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Ok(code_id) = CodeId::parse(&raw) else {
        return bootstrap(&raw);
    };

    let Some(access) = capability::resolve(&state, &code_id, &headers).await else {
        return bootstrap(code_id.as_str());
    };

    let binding = match ensure_binding(&state, &access.code, access.binding).await {
        Ok(binding) => binding,
        Err(error) => return store_response(&error),
    };

    let Some(binding) = binding else {
        return chooser(&state, &code_id).into_response();
    };

    let Some(handler) = state.registry().by_name(&binding.handler_id) else {
        // A binding to a handler this build no longer carries. Not an error the visitor
        // can do anything about, and not something to render a blank page for.
        tracing::warn!(
            handler = binding.handler_id,
            code = %code_id,
            "a code is bound to a handler this build does not have"
        );
        return (
            StatusCode::NOT_IMPLEMENTED,
            "this code was made by a newer version of the service\n",
        )
            .into_response();
    };

    let ctx = state.handler_ctx();
    match handler.render(&ctx, &binding).await {
        Err(error) => handler_response(&error),
        Ok(HandlerPage::Response(response)) => response,
        Ok(HandlerPage::Markup(body)) => {
            let described = handler
                .describe(&ctx, &binding)
                .await
                .unwrap_or_else(|_| json!({}));
            shell::render(
                Page::new(
                    binding
                        .title
                        .clone()
                        .unwrap_or_else(|| handler.name().to_owned()),
                    body,
                )
                .with_state(json!({
                    "code": code_id.as_str(),
                    "handler": binding.handler_id,
                    "state": described,
                }))
                .with_head(handler.head()),
            )
            .into_response()
        }
    }
}

/// Binds a code that was minted with a destination, the first time it is scanned.
///
/// A sheet printed for photo albums goes straight into an empty album; a sheet printed
/// blank shows the chooser instead. Losing the race against another phone is fine: the
/// unique index refuses the second insert and the winner's binding is read back.
async fn ensure_binding(
    state: &AppState,
    code: &qr_core::Code,
    existing: Option<Binding>,
) -> Result<Option<Binding>, qr_core::Error> {
    if existing.is_some() {
        return Ok(existing);
    }
    let Some(handler_id) = code.intended_handler.as_deref() else {
        return Ok(None);
    };
    if state.registry().by_name(handler_id).is_none() {
        return Ok(None);
    }

    match bind(
        state,
        &code.id,
        handler_id,
        code.intended_config.clone(),
        None,
    )
    .await
    {
        Ok(binding) => Ok(Some(binding)),
        Err(e) if e.is_conflict() => store::active_binding(state.store().pool(), &code.id).await,
        Err(e) => Err(e),
    }
}

/// Creates a binding and gives the handler its chance to write alongside it.
async fn bind(
    state: &AppState,
    code_id: &CodeId,
    handler_id: &str,
    config: serde_json::Value,
    title: Option<&str>,
) -> Result<Binding, qr_core::Error> {
    let mut tx = state.store().begin().await?;
    let binding = store::insert_binding(&mut *tx, code_id, handler_id, &config, title).await?;

    if let Some(handler) = state.registry().by_name(handler_id)
        && let Err(error) = handler.on_bind(&mut tx, &binding).await
    {
        return Err(qr_core::Error::Invalid(error.to_string()));
    }

    store::record_event(
        &mut *tx,
        EventKind::CodeBound,
        Some(code_id),
        Some(binding.id),
        &json!({ "handler": handler_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(binding)
}

/// The page shown before a cookie exists.
///
/// Its script reads `location.hash`, exchanges it for a cookie and replaces the history
/// entry so the secret never stays in the address bar. Without script it is the typed
/// form, which is also the damaged-sticker path and the reason the secret is printed.
/// `GET /C/{code}` — where a scanned sticker actually lands.
///
/// A QR encodes the whole URL uppercase, because alphanumeric mode has no lowercase and
/// byte mode would cost a version on every sticker. A browser lowercases the scheme and
/// the host on its own but leaves the path alone, so a scan arrives here rather than at
/// the route above.
///
/// Redirected rather than served from both spellings: the page's script rewrites the
/// address to `location.pathname` once it has redeemed, then reloads it, so the address
/// a visitor arrives at has to be the one the router answers.
///
/// The secret is in the fragment, which a browser reapplies to the destination of a
/// redirect that carries none of its own, so nothing is lost on the way.
pub async fn shouted(Path(raw): Path<String>) -> Response {
    let Ok(code) = CodeId::parse(&raw) else {
        // Nothing to normalise it to, and the typed form is the way back in.
        return bootstrap(&raw);
    };

    (
        StatusCode::PERMANENT_REDIRECT,
        [(header::LOCATION, format!("/c/{code}"))],
    )
        .into_response()
}

fn bootstrap(code: &str) -> Response {
    shell::render(
        Page::new(
            "Opening",
            html! {
                main class="bootstrap" x-data="bootstrap" {
                    h1 { "Opening this code" }
                    p class="only-with-script" { "One moment." }
                    noscript {
                        p { "Type what is printed under the QR code." }
                    }
                    form method="post" action="/go" class="go-form" {
                        label {
                            "Code"
                            input name="code" value=(code) autocapitalize="characters"
                                autocomplete="off" spellcheck="false" required;
                        }
                        label {
                            "Secret"
                            input name="secret" autocapitalize="characters"
                                autocomplete="off" spellcheck="false" required;
                        }
                        button type="submit" { "Open" }
                    }
                }
            },
        )
        .with_state(json!({ "redeem": code })),
    )
    .into_response()
}

/// What an unbound code offers.
fn chooser(state: &AppState, code: &CodeId) -> impl IntoResponse {
    let cards: Vec<_> = state
        .registry()
        .iter()
        .map(|h| (h.id(), h.chooser()))
        .collect();

    shell::render(
        Page::new(
            "What is this?",
            html! {
                main class="chooser" {
                    h1 { "What should this code be?" }
                    p class="code-id" { (code.grouped()) }
                    ul class="cards" {
                        @for (id, card) in &cards {
                            li {
                                form method="post" action={ "/c/" (code) "/api/v1/claim" } {
                                    input type="hidden" name="handler" value=(id.as_str());
                                    button type="submit" {
                                        span class="icon" { (card.icon) }
                                        span class="title" { (card.title) }
                                        span class="blurb" { (card.blurb) }
                                    }
                                }
                            }
                        }
                    }
                }
            },
        )
        .with_state(json!({
            "code": code.as_str(),
            "handlers": cards.iter().map(|(id, card)| json!({
                "id": id.as_str(),
                "title": card.title,
                "blurb": card.blurb,
            })).collect::<Vec<_>>(),
        })),
    )
}

#[derive(Deserialize)]
pub struct RedeemBody {
    pub secret: String,
}

/// `POST /c/{code}/api/v1/redeem` — the fragment, exchanged for a cookie.
pub async fn redeem(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Json(body): Json<RedeemBody>,
) -> Response {
    let (Ok(code_id), Ok(secret)) = (CodeId::parse(&raw), Secret::parse(&body.secret)) else {
        return refuse();
    };

    match store::redeem(state.store().pool(), &code_id, &secret).await {
        Err(error) => store_response(&error),
        Ok(None) => {
            // Logged without the code, so a log is not a list of ids worth trying.
            tracing::info!("a capability was refused");
            refuse()
        }
        Ok(Some(_)) => {
            if store::mark_first_seen(state.store().pool(), &code_id)
                .await
                .unwrap_or(false)
            {
                let _ = store::record_event(
                    state.store().pool(),
                    EventKind::CodeFirstSeen,
                    Some(&code_id),
                    None,
                    &json!({}),
                )
                .await;
            }

            let cookies = state.cookies();
            (
                StatusCode::NO_CONTENT,
                [(
                    header::SET_COOKIE,
                    capability::set_cookie(&code_id, &secret, cookies.max_age_days, cookies.secure),
                )],
            )
                .into_response()
        }
    }
}

fn refuse() -> Response {
    (
        StatusCode::NOT_FOUND,
        "this link is not valid. Scan the sticker on the box.\n",
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct ClaimBody {
    pub handler: String,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub title: Option<String>,
}

/// `POST /c/{code}/api/v1/claim`
pub async fn claim(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    body: ClaimForm,
) -> Response {
    let body = body.0;
    if access.binding.is_some() {
        return (StatusCode::CONFLICT, "this code is already claimed\n").into_response();
    }

    let Some(handler) = state.registry().by_name(&body.handler) else {
        return (StatusCode::BAD_REQUEST, "no such handler\n").into_response();
    };

    let config = match handler
        .validate_config(body.config.unwrap_or_else(|| handler.default_config()))
        .await
    {
        Ok(config) => config,
        Err(error) => return handler_response(&error),
    };

    match bind(
        &state,
        &access.code.id,
        &body.handler,
        config,
        body.title.as_deref(),
    )
    .await
    {
        Ok(_) => see_other(&format!("/c/{}", access.code.id)),
        Err(error) => store_response(&error),
    }
}

/// `POST /c/{code}/api/v1/release`
pub async fn release(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let Some(binding) = access.binding else {
        return (StatusCode::CONFLICT, "this code is not claimed\n").into_response();
    };

    let result = async {
        let mut tx = state.store().begin().await?;
        if let Some(handler) = state.registry().by_name(&binding.handler_id)
            && let Err(error) = handler.on_release(&mut tx, &binding).await
        {
            return Err(qr_core::Error::Invalid(error.to_string()));
        }
        store::release_binding(&mut *tx, binding.id).await?;
        store::record_event(
            &mut *tx,
            EventKind::CodeReleased,
            Some(&access.code.id),
            Some(binding.id),
            &json!({}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => see_other(&format!("/c/{}", access.code.id)),
        Err(error) => store_response(&error),
    }
}

/// `GET /c/{code}/api/v1/state` — the same payload the page hydrates from.
pub async fn code_state(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let Some(binding) = access.binding else {
        return Json(json!({
            "code": access.code.id.as_str(),
            "bound": false,
            "handlers": state.registry().iter().map(|h| json!({
                "id": h.id().as_str(),
                "title": h.chooser().title,
            })).collect::<Vec<_>>(),
        }))
        .into_response();
    };

    let Some(handler) = state.registry().by_name(&binding.handler_id) else {
        return (StatusCode::NOT_IMPLEMENTED, "unknown handler\n").into_response();
    };

    match handler.describe(&state.handler_ctx(), &binding).await {
        Ok(described) => Json(json!({
            "code": access.code.id.as_str(),
            "bound": true,
            "handler": binding.handler_id,
            "title": binding.title,
            "state": described,
        }))
        .into_response(),
        Err(error) => handler_response(&error),
    }
}

/// 303 so a form post does not stay in the history as a repeatable action.
fn see_other(location: &str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

/// A claim arriving as either a form post or JSON.
///
/// Both, because the chooser works without script and the client sends JSON. One
/// extractor rather than two routes, so there is one code path to get right.
pub struct ClaimForm(pub ClaimBody);

impl<S: Send + Sync> axum::extract::FromRequest<S> for ClaimForm {
    type Rejection = Response;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let json = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));

        if json {
            Json::<ClaimBody>::from_request(request, state)
                .await
                .map(|Json(body)| Self(body))
                .map_err(IntoResponse::into_response)
        } else {
            axum::extract::Form::<ClaimBody>::from_request(request, state)
                .await
                .map(|axum::extract::Form(body)| Self(body))
                .map_err(IntoResponse::into_response)
        }
    }
}
