//! The routes every code has, whatever it became.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Form, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use maud::html;
use qr_core::{Binding, CodeId, Location, NewLocation, Secret, model::EventKind, store};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    HandlerPage,
    capability::{self, CodeCtx},
    handler::Delegate,
    shell::{self, Page},
    state::AppState,
};

/// How many links may follow one another before the chain is called a loop.
///
/// A handful is more than a box with stickers on two sides needs; a chain this long is
/// either a cycle built by releasing and reclaiming, or someone having a laugh.
const MAX_LINK_HOPS: u32 = 8;

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
    if error.is_not_found() {
        return (StatusCode::NOT_FOUND, "nothing here\n").into_response();
    }
    if error.is_conflict() {
        return (StatusCode::CONFLICT, "that already happened\n").into_response();
    }
    if error.is_invalid() {
        return (StatusCode::BAD_REQUEST, format!("{error}\n")).into_response();
    }
    tracing::error!(%error, "a request could not reach the database");
    (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
}

/// How a link counts the hops it has already taken.
///
/// Only present while a chain of links is being followed; the page that finally
/// renders sends the visitor back to the untagged URL.
#[derive(Debug, Default, Deserialize)]
pub struct Hop {
    #[serde(default)]
    pub hop: Option<u32>,
}

/// Sends a scan of one sticker to another, with the other code's capability.
///
/// The cookie is what makes the destination work without asking again: the visitor
/// proved the link in the fragment, and only this service knows the target's secret. It
/// is scoped to the target's path like any other, so both codes keep their own cookie.
fn hand_off(state: &AppState, from: &CodeId, next: Delegate, hops: u32) -> Response {
    if hops >= MAX_LINK_HOPS {
        return looped(from);
    }

    let cookies = state.cookies();
    let location = format!("/c/{}?hop={}", next.code, hops + 1);

    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, location),
            (
                header::SET_COOKIE,
                capability::set_cookie(
                    &next.code,
                    &next.secret,
                    cookies.max_age_days,
                    cookies.secure,
                ),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
    )
        .into_response()
}

fn looped(from: &CodeId) -> Response {
    (
        StatusCode::LOOP_DETECTED,
        shell::render(Page::new(
            "Links in a loop",
            html! {
                main class="go" {
                    h1 { "These codes keep pointing at each other" }
                    p {
                        "Code "
                        span class="code-id" { (from.grouped()) }
                        " is part of a chain of links that never reaches a page."
                    }
                    p { "Release one of the links, then scan it again." }
                    p { a href="/" { "Back" } }
                }
            },
        )),
    )
        .into_response()
}

/// The bar every issued code carries.
///
/// Release is a soft delete and is offered with the consequence spelled out rather than
/// as a bare button: the photos and the destination survive, and the next scan offers
/// them back. Scripted visitors get the dialog; `<noscript>` gets the same form and the
/// same sentence, because a control that only works with script would be a second-class
/// way to lose a box.
fn controls(code: &CodeId, managing: bool, location: Option<&Location>) -> maud::Markup {
    html! {
        // The dialog is a sibling of the bar rather than a child. The bar has a
        // backdrop-filter, which makes it the containing block for fixed descendants; a
        // modal inside it would be fixed to the bar, not the viewport. The wrapper is
        // what shares the Alpine component between the two.
        div x-data="codeControls" {
            nav class="code-controls" aria-label="This code" {
                @if managing {
                    a class="control" href={ "/c/" (code) } { "Open" }
                } @else {
                    a class="control" href={ "/c/" (code) "/manage" } { "Manage" }
                }
                a class="control" href={ "/c/" (code) "/location" } { "Location" }
                @if let Some(location) = location {
                    span class="tagged" { (location.name) }
                }
                button type="button" class="control" x-on:click="openRelease" { "Release" }

                noscript {
                    form class="noscript-release" method="post"
                        action={ "/c/" (code) "/api/v1/release" } {
                        span { "Release this code? Its photos and history are kept." }
                        button type="submit" { "Release this code" }
                    }
                }
            }

            // A template rather than a hidden div: if the script never runs, the dialog
            // is not in the document at all. A modal that leaks into the page when Alpine
            // fails is worse than no button.
            template "x-if"="releaseOpen" {
                div class="modal-backdrop" {
                    div class="modal" role="dialog" aria-modal="true"
                        aria-labelledby="release-title" {
                        h2 id="release-title" { "Release this code?" }
                        p {
                            "The photos, link or destination stay in the database. Scanning
                             this sticker again offers them back under Recover QR code."
                        }
                        div class="modal-actions" {
                            button type="button" x-on:click="closeRelease" { "Cancel" }
                            form method="post" action={ "/c/" (code) "/api/v1/release" } {
                                button type="submit" class="danger" { "Release this code" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// `GET /c/{code}/manage` — the code administered rather than used.
///
/// A redirecting code sends every scan onwards, so this is where its destination can be
/// changed, and where release is reachable at all.
pub async fn manage(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let Some(binding) = access.binding else {
        return see_other(&format!("/c/{}", access.code.id));
    };

    let Some(handler) = state.registry().by_name(&binding.handler_id) else {
        return (StatusCode::NOT_IMPLEMENTED, "unknown handler\n").into_response();
    };

    let ctx = state.handler_ctx();
    match handler.manage(&ctx, &binding).await {
        Err(error) => handler_response(&error),
        Ok(HandlerPage::Response(response)) => response,
        Ok(HandlerPage::Markup(body)) => {
            let described = handler
                .describe(&ctx, &binding)
                .await
                .unwrap_or_else(|_| json!({}));
            let location = store::code_location(state.store().pool(), &access.code.id)
                .await
                .unwrap_or(None);
            shell::render(
                Page::new(
                    binding
                        .title
                        .clone()
                        .unwrap_or_else(|| handler.name().to_owned()),
                    html! {
                        (controls(&access.code.id, true, location.as_ref()))
                        (body)
                    },
                )
                .with_state(json!({
                    "code": access.code.id.as_str(),
                    "handler": binding.handler_id,
                    "state": described,
                })),
            )
            .into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct RecoverBody {
    pub binding: Uuid,
}

/// `POST /c/{code}/api/v1/recover` — put a released use back.
///
/// The row was never deleted, so this is one UPDATE. What it must not do is let two
/// active bindings exist: the partial unique index refuses that, and a code that was
/// claimed in the meantime gets a conflict rather than a silent swap.
pub async fn recover(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Form(body): Form<RecoverBody>,
) -> Response {
    let result = async {
        let mut tx = state.store().begin().await?;

        let Some(binding) = store::binding(&mut *tx, &access.code.id, body.binding).await? else {
            return Err(qr_core::Error::NotFound("binding"));
        };
        if binding.released_at.is_none() {
            return Err(qr_core::Error::Conflict("binding"));
        }
        if store::active_binding(&mut *tx, &access.code.id)
            .await?
            .is_some()
        {
            return Err(qr_core::Error::Conflict("binding"));
        }
        if !store::recover_binding(&mut *tx, binding.id).await? {
            return Err(qr_core::Error::Conflict("binding"));
        }

        store::record_event(
            &mut *tx,
            EventKind::CodeRecovered,
            Some(&access.code.id),
            Some(binding.id),
            &json!({ "handler": binding.handler_id }),
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

#[derive(Deserialize)]
pub struct LocationBody {
    #[serde(default)]
    pub location: Option<Uuid>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

/// `GET /c/{code}/location` — where this sticker is, and how to say so.
///
/// A page rather than a dialog: the catalog has to be rendered somewhere, and a native
/// `<select>` beats a dropdown built from script. The device-location half needs script;
/// picking an existing place does not.
pub async fn location_page(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    if access.binding.is_none() {
        return see_other(&format!("/c/{}", access.code.id));
    }

    let current = store::code_location(state.store().pool(), &access.code.id)
        .await
        .unwrap_or(None);
    let catalog = store::locations(state.store().pool())
        .await
        .unwrap_or_default();
    let code = &access.code.id;

    shell::render(
        Page::new(
            "Location",
            html! {
                (controls(code, false, current.as_ref()))
                main class="location" {
                    h1 { "Where is this code?" }
                    p class="code-id" { (code.grouped()) }

                    @match &current {
                        Some(place) => p class="current-location" {
                            "Tagged as " strong { (place.name) }
                            @if let Some(address) = &place.address {
                                " — " (address)
                            }
                        },
                        None => p class="empty" { "Not tagged yet." },
                    }

                    section class="place-new" {
                        h2 { "Use this device's location" }
                        form method="post" action={ "/c/" (code) "/api/v1/location" }
                            x-data="locationTag" {
                            label {
                                "Name"
                                input name="name" required maxlength="80"
                                    autocomplete="off" spellcheck="false"
                                    placeholder="Loft, shelf 3";
                            }
                            label {
                                "Address (optional)"
                                input name="address" maxlength="200" autocomplete="off"
                                    spellcheck="false";
                            }
                            input type="hidden" name="lat" x-model="lat";
                            input type="hidden" name="lon" x-model="lon";

                            div class="locate" {
                                button type="button" x-on:click="locate" {
                                    "Use my location"
                                }
                                span class="hint" x-text="status" {}
                            }
                            button type="submit" disabled x-bind:disabled="noCoords" {
                                "Tag this place"
                            }
                            noscript {
                                p class="hint" {
                                    "Getting the device's position needs JavaScript. Pick an
                                     existing place below instead."
                                }
                            }
                        }
                    }

                    @if !catalog.is_empty() {
                        section class="place-existing" {
                            h2 { "Choose an existing place" }
                            form method="post" action={ "/c/" (code) "/api/v1/location" } {
                                label {
                                    "Place"
                                    select name="location" required {
                                        @for place in &catalog {
                                            option value=(place.id) {
                                                (place.name)
                                                @if let Some(address) = &place.address {
                                                    " — " (address)
                                                }
                                            }
                                        }
                                    }
                                }
                                button type="submit" { "Tag" }
                            }
                        }
                    }

                    @if current.is_some() {
                        form class="untag" method="post"
                            action={ "/c/" (code) "/api/v1/location/clear" } {
                            button type="submit" { "Remove tag" }
                        }
                    }
                }
            },
        )
        .with_state(json!({ "code": code.as_str() })),
    )
    .into_response()
}

/// `POST /c/{code}/api/v1/location` — attach an existing place, or create one.
pub async fn tag_location(
    CodeCtx(access): CodeCtx,
    State(state): State<AppState>,
    Form(body): Form<LocationBody>,
) -> Response {
    let result = async {
        let mut tx = state.store().begin().await?;

        let place = match body.location {
            Some(id) => store::location(&mut *tx, id)
                .await?
                .ok_or_else(|| qr_core::Error::Invalid("no such place".to_owned()))?,
            None => {
                let name = body.name.as_deref().map(str::trim).unwrap_or_default();
                if name.is_empty() || name.chars().count() > 80 {
                    return Err(qr_core::Error::Invalid(
                        "a place needs a name of at most 80 characters".to_owned(),
                    ));
                }

                let address = body
                    .address
                    .as_deref()
                    .map(str::trim)
                    .filter(|address| !address.is_empty());
                if address.is_some_and(|address| address.chars().count() > 200) {
                    return Err(qr_core::Error::Invalid(
                        "that address is too long".to_owned(),
                    ));
                }

                let (Some(lat), Some(lon)) = (body.lat, body.lon) else {
                    return Err(qr_core::Error::Invalid(
                        "a new place needs coordinates from this device".to_owned(),
                    ));
                };
                if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
                    return Err(qr_core::Error::Invalid(
                        "latitude must be between -90 and 90".to_owned(),
                    ));
                }
                if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
                    return Err(qr_core::Error::Invalid(
                        "longitude must be between -180 and 180".to_owned(),
                    ));
                }

                store::insert_location(
                    &mut *tx,
                    NewLocation {
                        name: name.to_owned(),
                        address: address.map(str::to_owned),
                        lat,
                        lon,
                    },
                )
                .await?
            }
        };

        store::set_code_location(&mut *tx, &access.code.id, Some(place.id)).await?;
        store::record_event(
            &mut *tx,
            EventKind::LocationTagged,
            Some(&access.code.id),
            access.binding.as_ref().map(|binding| binding.id),
            &json!({ "location": place.id, "name": place.name }),
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

/// `POST /c/{code}/api/v1/location/clear` — the sticker is no longer anywhere in
/// particular.
pub async fn clear_location(CodeCtx(access): CodeCtx, State(state): State<AppState>) -> Response {
    let result = async {
        let mut tx = state.store().begin().await?;
        store::set_code_location(&mut *tx, &access.code.id, None).await?;
        store::record_event(
            &mut *tx,
            EventKind::LocationTagged,
            Some(&access.code.id),
            access.binding.as_ref().map(|binding| binding.id),
            &json!({ "cleared": true }),
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

/// `GET /c/{code}/api/v1/locations` — the catalog, for a script that would rather pick
/// from JSON than parse the page.
pub async fn list_locations(State(state): State<AppState>) -> Response {
    match store::locations(state.store().pool()).await {
        Ok(locations) => Json(json!({ "locations": locations })).into_response(),
        Err(error) => store_response(&error),
    }
}

/// `GET /c/{code}` — the page a scan lands on.
///
/// Unguarded on purpose: this is the one route that has to answer before a cookie
/// exists, because the secret is still in the fragment and only a browser can see it.
pub async fn landing(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Query(hop): Query<Hop>,
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
        if hop.hop.is_some() {
            // A link sent the visitor here; this code is not itself a link, so the
            // hop counter has done its job and does not belong in the address bar.
            return see_other(&format!("/c/{code_id}"));
        }
        return chooser(&state, &code_id).await;
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

    match handler.delegate(&ctx, &binding).await {
        Err(error) => return handler_response(&error),
        Ok(Some(next)) => return hand_off(&state, &code_id, next, hop.hop.unwrap_or(0)),
        Ok(None) => {}
    }

    if hop.hop.is_some() {
        return see_other(&format!("/c/{code_id}"));
    }

    match handler.render(&ctx, &binding).await {
        Err(error) => handler_response(&error),
        Ok(HandlerPage::Response(response)) => response,
        Ok(HandlerPage::Markup(body)) => {
            let described = handler
                .describe(&ctx, &binding)
                .await
                .unwrap_or_else(|_| json!({}));
            let location = store::code_location(state.store().pool(), &code_id)
                .await
                .unwrap_or(None);
            shell::render(
                Page::new(
                    binding
                        .title
                        .clone()
                        .unwrap_or_else(|| handler.name().to_owned()),
                    html! {
                        (controls(&code_id, false, location.as_ref()))
                        (body)
                    },
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
///
/// A released code keeps its history, and this is where that history is offered back.
/// Recovery is not a restore of a deleted thing: the binding row was only marked
/// released, and the photos, destination and link row behind it never moved.
async fn chooser(state: &AppState, code: &CodeId) -> Response {
    let cards: Vec<_> = state
        .registry()
        .iter()
        .map(|h| (h.id(), h.chooser()))
        .collect();

    let released = store::released_bindings(state.store().pool(), code)
        .await
        .unwrap_or_default();
    let ctx = state.handler_ctx();
    let mut history = Vec::new();
    for binding in released {
        let Some(handler) = state.registry().by_name(&binding.handler_id) else {
            // A binding to a handler this build no longer carries. It cannot be
            // recovered into a page, but it stays in the table.
            continue;
        };
        let summary = handler
            .describe(&ctx, &binding)
            .await
            .unwrap_or_else(|_| json!({}));
        history.push((binding, handler.name(), summary));
    }

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
                                form class=(if card.fields.0.is_empty() { "" } else { "has-fields" })
                                    method="post" action={ "/c/" (code) "/api/v1/claim" } {
                                    input type="hidden" name="handler" value=(id.as_str());
                                    @if card.fields.0.is_empty() {
                                        button type="submit" {
                                            span class="icon" { (card.icon) }
                                            span class="title" { (card.title) }
                                            span class="blurb" { (card.blurb) }
                                        }
                                    } @else {
                                        div class="card-body" {
                                            span class="icon" { (card.icon) }
                                            span class="title" { (card.title) }
                                            span class="blurb" { (card.blurb) }
                                        }
                                        (card.fields)
                                        button type="submit" class="card-claim" { "Use this" }
                                    }
                                }
                            }
                        }
                    }

                    @if !history.is_empty() {
                        section class="recover" {
                            h2 { "Recover QR code" }
                            p class="hint" {
                                "This sticker has been used before. Recovering one of these keeps
                                 what it held, including its photos."
                            }
                            ul class="recover-list" {
                                @for (binding, name, summary) in &history {
                                    li {
                                        div class="recover-what" {
                                            strong {
                                                (binding.title.as_deref().unwrap_or(name))
                                            }
                                            span class="blurb" {
                                                (describe_line(name, summary, binding.released_at))
                                            }
                                        }
                                        form method="post"
                                            action={ "/c/" (code) "/api/v1/recover" } {
                                            input type="hidden" name="binding"
                                                value=(binding.id);
                                            button type="submit" { "Recover" }
                                        }
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
    .into_response()
}

/// One line saying what a released binding was, without opening it.
fn describe_line(
    handler: &str,
    summary: &serde_json::Value,
    released_at: Option<DateTime<Utc>>,
) -> String {
    let mut parts = vec![handler.to_owned()];

    if let Some(total) = summary.get("total").and_then(serde_json::Value::as_u64) {
        parts.push(match total {
            1 => "1 photo".to_owned(),
            n => format!("{n} photos"),
        });
    }
    if let Some(url) = summary.get("url").and_then(serde_json::Value::as_str) {
        parts.push(format!("→ {url}"));
    } else if let Some(code) = summary.get("code").and_then(serde_json::Value::as_str) {
        parts.push(format!("→ {}", display_code(code)));
    }
    if let Some(at) = released_at {
        parts.push(format!("released {}", at.format("%-d %b %Y")));
    }

    parts.join(" · ")
}

fn display_code(raw: &str) -> String {
    CodeId::parse(raw)
        .map(|code| code.grouped())
        .unwrap_or_else(|_| raw.to_owned())
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
        .validate_config(
            &state.handler_ctx(),
            body.config.unwrap_or_else(|| handler.default_config()),
        )
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
///
/// A form cannot express a nested object, so a field named `config.code` becomes
/// `{"code": …}`. That is what lets a handler put its own inputs on the chooser card and
/// still be claimed by a browser with no script.
pub struct ClaimForm(pub ClaimBody);

impl ClaimBody {
    fn from_pairs(pairs: HashMap<String, String>) -> Self {
        let mut handler = String::new();
        let mut title = None;
        let mut config = serde_json::Map::new();

        for (key, value) in pairs {
            match key.as_str() {
                "handler" => handler = value,
                "title" if !value.is_empty() => title = Some(value),
                _ => {
                    if let Some(name) = key.strip_prefix("config.") {
                        config.insert(name.to_owned(), serde_json::Value::String(value));
                    }
                }
            }
        }

        Self {
            handler,
            config: (!config.is_empty()).then_some(serde_json::Value::Object(config)),
            title,
        }
    }
}

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
            axum::extract::Form::<HashMap<String, String>>::from_request(request, state)
                .await
                .map(|axum::extract::Form(pairs)| Self(ClaimBody::from_pairs(pairs)))
                .map_err(IntoResponse::into_response)
        }
    }
}
