//! The seam between what the service is and what a code does.
//!
//! A code is an anchor; a handler is something that claims one and owns everything
//! under its prefix. Adding a third handler is a crate that depends on this one, not an
//! edit to the core, which is the whole reason this crate is separate from the binary.

pub mod capability;
pub mod handler;
pub mod id;
pub mod registry;
pub mod routes;
pub mod shell;
pub mod state;

pub use capability::{CodeAccess, CodeCtx};
pub use handler::{ChooserCard, Delegate, Handler, HandlerError, HandlerPage};
pub use id::HandlerId;
pub use registry::{Registry, RegistryBuilder};
pub use shell::{Page, render as render_page};
pub use state::{AppState, CookieConfig, HandlerCtx, Limits};

use axum::{
    Router,
    routing::{get, post},
};

/// Builds the whole `/c` subtree.
///
/// Everything a code owns lives under `/c/{code}`, including its slice of the API, so
/// one cookie scoped to that path covers pages, API calls and image loads alike. The
/// alternative — an API at `/api/v1/codes/{code}` — would not receive that cookie at
/// all, and widening the cookie to `Path=/` would send every code's secret on every
/// request to everything.
pub fn mount(state: AppState) -> Router {
    // The two routes that have to answer before a cookie exists. The landing page is
    // where the fragment is still readable, and redeem is what turns it into a cookie.
    let open = Router::new()
        .route("/c/{code}", get(routes::landing))
        // What a scan actually requests; see routes::shouted.
        .route("/C/{code}", get(routes::shouted))
        .route("/c/{code}/api/v1/redeem", post(routes::redeem));

    let mut guarded = Router::new()
        .route("/c/{code}/api/v1/state", get(routes::code_state))
        .route("/c/{code}/api/v1/claim", post(routes::claim))
        .route("/c/{code}/api/v1/release", post(routes::release));

    for handler in state.registry().iter() {
        // A literal segment rather than a `{handler}` parameter, so an unknown handler
        // is a 404 in the router instead of a lookup that has to decide what to say.
        let prefix = format!("/c/{{code}}/api/v1/h/{}", handler.id());
        guarded = guarded.nest(&prefix, handler.routes());

        // And the handler's pages, a sibling of the API so a browser can navigate to one
        // without it looking like JSON.
        let pages = format!("/c/{{code}}/h/{}", handler.id());
        guarded = guarded.nest(&pages, handler.pages());
    }

    // route_layer, not layer: a path matching nothing in this subtree should 404 in the
    // router without paying for a database lookup, and a miss must never be reported as
    // an authorisation failure.
    let guarded = guarded.route_layer(axum::middleware::from_fn_with_state(
        state.clone(),
        capability::require_capability,
    ));

    open.merge(guarded).with_state(state)
}
