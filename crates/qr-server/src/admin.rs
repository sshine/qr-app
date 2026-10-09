//! The admin API.
//!
//! One route so far, and it mints. Everything here is behind [`RequireAdmin`] rather
//! than behind the capability cookie: a sheet is minted before any of its codes exist,
//! so there is no code to hold a capability for.

use std::{path::Path, sync::Arc};

use axum::{
    Json, Router,
    extract::{FromRef, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use qr_handler::{AdminConfig, AdminIdentity, AppState, RequireAdmin};
use qr_sheet::{EcLevel, Presets, SheetRenderer};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::{StartupError, cli::ServeArgs, sheet};

/// The most codes one request may ask for.
///
/// A sheet costs roughly 3.5ms per code end to end, so this is a few seconds of work and
/// a couple of megabytes of PDF held in memory. The ceiling exists so a typo in `count`
/// cannot run past the service's own 30-second request timeout and be answered with
/// nothing after the codes have already been minted.
const MAX_CODES: usize = 1000;

/// What the admin routes need on top of [`AppState`].
#[derive(Clone)]
pub struct AdminState {
    app: AppState,
    sheets: Arc<Sheets>,
}

/// Rendering a sheet forks typst, which peaks around 31MB resident and holds a core for
/// the duration. Bounded rather than one process per request: the pod's memory ceiling
/// is sized for a decoded photo plus a compile, and it is what gives way first when
/// several sheets are asked for at once.
struct Sheets {
    renderer: SheetRenderer,
    presets: Presets,
    ec: EcLevel,
    id_len: usize,
    secret_len: usize,
    permits: Semaphore,
}

impl FromRef<AdminState> for AdminConfig {
    fn from_ref(state: &AdminState) -> Self {
        Self::from_ref(&state.app)
    }
}

/// Builds the state the admin routes share.
pub fn state(args: &ServeArgs, app: AppState) -> Result<AdminState, StartupError> {
    Ok(AdminState {
        app,
        sheets: Arc::new(Sheets {
            renderer: SheetRenderer::new(
                args.typst.typst_bin.clone(),
                args.typst.typst_font_path.clone(),
            ),
            presets: sheet::load_presets(args.presets_file.as_deref())?,
            ec: qr_sheet::parse_ec_level(&args.codes.qr_ec_level)?,
            id_len: args.codes.code_id_len,
            secret_len: args.codes.code_secret_len,
            permits: Semaphore::new(args.sheet_concurrency.into()),
        }),
    })
}

/// Everything the admin API answers.
pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/sheets", post(create_sheet))
        .with_state(state)
}

/// What `POST /admin/sheets` accepts.
///
/// Unknown fields are refused rather than ignored, because a misspelled `handler` that
/// is quietly dropped is a sheet of codes that land on the chooser, discovered once they
/// are on boxes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSheet {
    /// Which label stock, by preset name.
    preset: String,
    /// Defaults to filling one sheet of that stock. More spills onto further pages.
    count: Option<usize>,
    /// What every code becomes on first scan. Omitted leaves the chooser.
    handler: Option<String>,
    /// Printed small at the foot, so two printings can be told apart later.
    note: Option<String>,
    /// Error-correction level. Defaults to what the service was started with.
    ec: Option<String>,
}

/// `POST /admin/sheets` — mints a sheet of codes and answers with the PDF.
///
/// The bytes are the whole response and are kept nowhere. They carry every secret on the
/// sheet in plaintext, so there is no second copy to expire, collect, or leak, and a
/// download that was lost is reprinted by minting another sheet.
async fn create_sheet(
    State(state): State<AdminState>,
    RequireAdmin(admin): RequireAdmin,
    Json(body): Json<NewSheet>,
) -> Response {
    let sheets = &state.sheets;

    let Ok(layout) = sheets.presets.get(&body.preset) else {
        return refused(format!(
            "no preset named {}; this build has {}\n",
            body.preset,
            sheets.presets.names().collect::<Vec<_>>().join(", "),
        ));
    };

    let ec = match body.ec.as_deref() {
        None => sheets.ec,
        Some(given) => match qr_sheet::parse_ec_level(given) {
            Ok(ec) => ec,
            Err(error) => return refused(format!("{error}\n")),
        },
    };

    // Checked against the registry, which the CLI does not do: a sheet naming a handler
    // this build has never heard of is codes that can never bind to anything.
    if let Some(handler) = &body.handler
        && state.app.registry().by_name(handler).is_none()
    {
        return refused(format!("no handler named {handler}\n"));
    }

    let count = body.count.unwrap_or_else(|| layout.per_page());
    if count == 0 || count > MAX_CODES {
        return refused(format!("count must be between 1 and {MAX_CODES}\n"));
    }

    // Held across the render rather than taken inside it, so a request that has to wait
    // waits here instead of in a queue of forked compilers.
    let Ok(_permit) = sheets.permits.acquire().await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response();
    };

    let rendered = sheet::mint_and_render(
        state.app.store(),
        &sheets.renderer,
        sheet::Order {
            base: state.app.base(),
            layout,
            preset: &body.preset,
            count,
            handler: body.handler.as_deref(),
            note: body.note.as_deref(),
            ec,
            id_len: sheets.id_len,
            secret_len: sheets.secret_len,
        },
    )
    .await;

    match rendered {
        Err(error) => sheet_response(&error),
        Ok(rendered) => {
            audit(&admin, rendered.sheet_id, count, &body);
            (
                StatusCode::CREATED,
                [
                    (header::CONTENT_TYPE, "application/pdf".to_owned()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"sheet-{}.pdf\"", rendered.sheet_id),
                    ),
                    // The response is the only copy, and it is a page of live secrets.
                    (header::CACHE_CONTROL, "no-store".to_owned()),
                ],
                rendered.pdf,
            )
                .into_response()
        }
    }
}

/// Who minted what, at info level, so a sheet that nobody admits to printing can be
/// traced to the door its request came through.
fn audit(admin: &AdminIdentity, sheet: uuid::Uuid, count: usize, body: &NewSheet) {
    tracing::info!(
        subject = %admin.subject,
        source = ?admin.source,
        %sheet,
        count,
        preset = %body.preset,
        handler = body.handler.as_deref().unwrap_or("-"),
        "a sheet was minted"
    );
}

fn refused(message: String) -> Response {
    (StatusCode::BAD_REQUEST, message).into_response()
}

/// Everything minting a sheet fails with.
///
/// The layout errors are a 400 rather than a 500: they say the codes will not fit the
/// stock that was asked for, which the caller fixes by asking for other stock.
fn sheet_response(error: &sheet::Error) -> Response {
    use qr_sheet::Error as E;

    match error {
        sheet::Error::Sheet(
            e @ (E::UnknownPreset(_)
            | E::SymbolTooLarge { .. }
            | E::CaptionTooWide { .. }
            | E::CaptionTooTall { .. }
            | E::OffPage { .. }),
        ) => refused(format!("{e}\n")),
        sheet::Error::Core(e) if e.is_invalid() => refused(format!("{e}\n")),
        sheet::Error::Core(e) => {
            tracing::error!(%e, "a sheet could not reach the database");
            (StatusCode::SERVICE_UNAVAILABLE, "try again\n").into_response()
        }
        sheet::Error::Sheet(e) => {
            tracing::error!(%e, "a sheet could not be rendered");
            (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong\n").into_response()
        }
    }
}

/// Reads the presets a deployment was given, so `--check-config` refuses a file that
/// does not parse rather than leaving it to the first print of the year.
pub fn check_presets(path: Option<&Path>) -> Result<(), StartupError> {
    sheet::load_presets(path).map(drop)
}
