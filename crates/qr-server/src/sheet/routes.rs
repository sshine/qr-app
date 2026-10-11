//! Minting a sheet of codes over HTTP.
//!
//! Open on purpose. A request here mints codes that did not exist a moment ago, each
//! with a secret of its own, and the response is the only copy of them; it reaches
//! nothing minted earlier and no code anyone is already holding. What it does cost is
//! database rows and a typst compile, which is what the ceiling and the permit below
//! are for.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use qr_handler::AppState;
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

/// What this route needs on top of [`AppState`].
#[derive(Clone)]
pub struct SheetState {
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

/// Builds the state this route needs.
pub fn state(args: &ServeArgs, app: AppState) -> Result<SheetState, StartupError> {
    Ok(SheetState {
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

/// Where minting answers.
pub fn router(state: SheetState) -> Router {
    Router::new()
        .route("/api/v1/sheets", get(open_sheet).post(create_sheet))
        .with_state(state)
}

/// What either verb accepts: a JSON body on POST, a query string on GET.
///
/// Unknown fields are refused rather than ignored, because a misspelled `handler` that
/// is quietly dropped is a sheet of codes that land on the chooser, discovered once they
/// are on boxes. That holds for a mistyped query parameter too.
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

/// `GET /api/v1/sheets?preset=relief-t7160&count=21` — the same sheet, from a link.
///
/// A GET that creates, which the method is not supposed to do. It earns the exception
/// because the thing being asked for is a printable page and the natural way to ask for
/// one is a URL somebody can open; without this the only way in is a shell. The price is
/// that every load mints: opening the link twice gives two sheets of different codes, and
/// a browser that prefetches the link has already spent one.
///
/// Rendered inline rather than downloaded, because a link is opened to be looked at.
async fn open_sheet(State(state): State<SheetState>, Query(query): Query<NewSheet>) -> Response {
    mint(&state, query, StatusCode::OK, "inline").await
}

/// `POST /api/v1/sheets` — the same, for a script that wants the bytes.
async fn create_sheet(State(state): State<SheetState>, Json(body): Json<NewSheet>) -> Response {
    mint(&state, body, StatusCode::CREATED, "attachment").await
}

/// Mints a sheet of codes and answers with the PDF.
///
/// The bytes are the whole response and are kept nowhere. They carry every secret on the
/// sheet in plaintext, so there is no second copy to expire, collect, or leak, and a
/// download that was lost is reprinted by minting another sheet.
async fn mint(
    state: &SheetState,
    body: NewSheet,
    success: StatusCode,
    disposition: &str,
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
            minted(rendered.sheet_id, count, &body);
            (
                success,
                [
                    (header::CONTENT_TYPE, "application/pdf".to_owned()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!(
                            "{disposition}; filename=\"sheet-{}.pdf\"",
                            rendered.sheet_id
                        ),
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

/// What was minted, at info level, so a sheet found on a desk months later can be
/// matched to the request that printed it.
fn minted(sheet: uuid::Uuid, count: usize, body: &NewSheet) {
    tracing::info!(
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
