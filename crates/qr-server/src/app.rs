//! Assembling the service.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use maud::html;
use qr_core::{BaseUrl, BlobStore, CodeId, Secret, Store};
use qr_handler::{
    AdminConfig, AppState, CookieConfig, Limits, Registry, capability,
    shell::{self, Page},
};
use serde::Deserialize;
use serde_json::json;
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    sensitive_headers::SetSensitiveRequestHeadersLayer, set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::{admin, assets, cli::ServeArgs};

/// No inline anything, and no 'unsafe-eval' — which is the whole reason Alpine's CSP
/// build is the one vendored. `blob:` is what lets a photo appear in the grid before
/// its upload has finished.
const CSP: &str = "default-src 'none'; img-src 'self' blob: data:; script-src 'self'; \
                   style-src 'self'; connect-src 'self'; form-action 'self'; \
                   frame-ancestors 'none'; base-uri 'none'; manifest-src 'self'";

/// Everything the public listener answers.
pub fn router(state: AppState, admin: admin::AdminState) -> Router {
    let pages = Router::new()
        .route("/", get(landing))
        .route("/go", get(go_form).post(go))
        .route("/manifest.webmanifest", get(manifest))
        .route("/sw.js", get(assets::service_worker))
        .with_state(state.clone());

    Router::new()
        .merge(qr_handler::mount(state))
        .merge(pages)
        .merge(admin::router(admin))
        .merge(assets::router())
        .fallback(not_found)
        .layer(
            tower::ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                // The cookie is the capability. TraceLayer's default span would print
                // it into the log of every request that carries one.
                .layer(SetSensitiveRequestHeadersLayer::new([
                    header::COOKIE,
                    header::AUTHORIZATION,
                ]))
                .layer(TraceLayer::new_for_http())
                .layer(SetResponseHeaderLayer::overriding(
                    header::REFERRER_POLICY,
                    HeaderValue::from_static("no-referrer"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(CSP),
                ))
                // 503 rather than the default, because a request that ran out of time
                // is this service failing to answer, not the caller asking wrongly.
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    Duration::from_secs(30),
                ))
                // Small by default; the upload route raises it for itself alone, so
                // nothing else can be made to buffer megabytes by asking.
                .layer(DefaultBodyLimit::max(64 * 1024))
                .layer(CompressionLayer::new().gzip(true).br(true)),
        )
}

/// Builds the state every route shares.
pub async fn state(
    args: &ServeArgs,
    store: Store,
    blobs: Arc<dyn BlobStore>,
    registry: Registry,
) -> Result<AppState, crate::StartupError> {
    let base = BaseUrl::parse(&args.base_url).map_err(crate::StartupError::BaseUrl)?;

    let admin_token = match &args.admin_token_file {
        None => None,
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|source| crate::StartupError::AdminToken {
                    path: path.clone(),
                    source,
                })?
                .trim()
                .to_owned(),
        ),
    };

    let admin = AdminConfig {
        header: args.admin_header.clone(),
        // Generated on first start and kept in the database, so it survives a restart
        // and rotates with one DELETE.
        key: store
            .signing_key()
            .await
            .map_err(crate::StartupError::Core)?,
        session_secs: u64::from(args.admin_session_hours) * 3600,
        static_token: admin_token.filter(|t| !t.is_empty()),
        verifier: None,
    };

    Ok(AppState::new(
        store,
        blobs,
        registry,
        base,
        CookieConfig {
            max_age_days: args.cookie_max_age_days,
            secure: !args.insecure_cookies,
        },
        Limits {
            max_upload_bytes: args.max_upload_bytes,
            max_upload_pixels: args.max_upload_pixels,
            strip_original_exif: args.strip_original_exif,
        },
        admin,
    ))
}

/// The page an installed app opens on, and the only one not reached by scanning.
///
/// The codes it lists come from the browser's own storage, because the service has no
/// idea who is asking and a list of every code would be the inventory the whole
/// capability scheme exists to withhold.
async fn landing() -> Response {
    shell::render(
        Page::new(
            "QR",
            html! {
                main class="landing" x-data="recent" {
                    h1 { "Scan a code" }
                    p { "Point a camera at the sticker on the box." }
                    p { a href="/go" { "Or type what is printed under it" } }
                    template x-if="codes.length" {
                        section {
                            h2 { "Opened recently" }
                            ul class="cards" {
                                template "x-for"="code in codes" {
                                    li { a "x-bind:href"="href" x-text="code" {} }
                                }
                            }
                        }
                    }
                }
            },
        )
        .with_state(json!({ "landing": true })),
    )
    .into_response()
}

/// The typed fallback, which is also what a visitor without script gets.
async fn go_form() -> Response {
    shell::render(Page::new(
        "Open a code",
        html! {
            main class="go" {
                h1 { "Open a code" }
                p { "Type what is printed under the QR code." }
                form method="post" action="/go" {
                    label {
                        "Code"
                        input name="code" autocapitalize="characters" autocomplete="off"
                            spellcheck="false" required;
                    }
                    label {
                        "Secret"
                        input name="secret" autocapitalize="characters" autocomplete="off"
                            spellcheck="false" required;
                    }
                    button type="submit" { "Open" }
                }
            }
        },
    ))
    .into_response()
}

#[derive(Deserialize)]
struct Go {
    code: String,
    secret: String,
}

/// Exchanges a typed id and secret for the same cookie the fragment would have bought.
///
/// A POST, so the secret is in a body rather than a query string that would be written
/// to every log between here and the browser.
async fn go(
    State(state): State<AppState>,
    axum::extract::Form(form): axum::extract::Form<Go>,
) -> Response {
    let (Ok(code), Ok(secret)) = (CodeId::parse(&form.code), Secret::parse(&form.secret)) else {
        return go_refused();
    };

    match qr_core::store::redeem(state.store().pool(), &code, &secret).await {
        Ok(Some(_)) => {
            let cookies = state.cookies();
            (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, format!("/c/{code}")),
                    (
                        header::SET_COOKIE,
                        capability::set_cookie(
                            &code,
                            &secret,
                            cookies.max_age_days,
                            cookies.secure,
                        ),
                    ),
                ],
            )
                .into_response()
        }
        Ok(None) => go_refused(),
        Err(error) => {
            tracing::error!(%error, "a typed code could not be checked");
            (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
        }
    }
}

/// The same answer for a wrong secret and an unknown code, for the same reason the
/// capability layer gives one.
fn go_refused() -> Response {
    (
        StatusCode::NOT_FOUND,
        shell::render(Page::new(
            "Not valid",
            html! {
                main class="go" {
                    h1 { "That did not open anything" }
                    p { "Check the characters under the QR code and try again." }
                    p { a href="/go" { "Back" } }
                }
            },
        )),
    )
        .into_response()
}

async fn manifest() -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        json!({
            "name": "QR",
            "short_name": "QR",
            "description": "Pre-printed QR stickers, and what they open",
            // The landing page, because an installed app has no sticker to scan from
            // and a code-specific start URL would pin it to one box.
            "start_url": "/",
            "scope": "/",
            "display": "standalone",
            "background_color": "#fbfaf8",
            "theme_color": "#16150f",
            "icons": [{
                "src": "/assets/icon.svg",
                "sizes": "any",
                "type": "image/svg+xml",
                "purpose": "any maskable",
            }],
        })
        .to_string(),
    )
        .into_response()
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        shell::render(Page::new(
            "Nothing here",
            html! {
                main { h1 { "Nothing here" } p { a href="/" { "Back" } } }
            },
        )),
    )
        .into_response()
}

/// Every handler this build carries.
///
/// Registration is refused on a duplicate id, so adding a third handler that collides
/// with one of these fails at startup rather than silently shadowing it.
pub fn registry() -> Result<Registry, qr_handler::registry::DuplicateHandler> {
    Ok(Registry::builder()
        .register(qr_photos::PhotoAlbum)?
        .register(qr_forward::ForwardHandler)?
        .build())
}
