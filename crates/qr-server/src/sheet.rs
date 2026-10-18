//! Minting a sheet of codes, from the CLI and over HTTP.
//!
//! Printing happens a handful of times a year. The CLI was the first way in and
//! `POST /api/v1/sheets` is the second; both go through [`mint_and_render`], so the two
//! cannot come to mint codes on different terms.

pub mod routes;

use std::path::Path;

use qr_core::{BaseUrl, MintedCode, Store, model::EventKind, store};
use qr_sheet::{EcLevel, Layout, Presets, SheetRenderer, SheetRequest};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::{
    StartupError,
    cli::{BudgetArgs, SheetArgs},
};

/// What minting a sheet fails with.
///
/// Narrower than [`StartupError`]: a request asking for stock the codes will not fit on
/// is not the service failing to start, and the route has to tell the two apart to pick
/// a status code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sheet(#[from] qr_sheet::Error),

    #[error(transparent)]
    Core(#[from] qr_core::Error),
}

impl From<Error> for StartupError {
    fn from(error: Error) -> Self {
        match error {
            Error::Sheet(e) => Self::Sheet(e),
            Error::Core(e) => Self::Core(e),
        }
    }
}

/// What to mint, however the request arrived.
pub struct Order<'a> {
    pub base: &'a BaseUrl,
    pub layout: &'a Layout,
    pub preset: &'a str,
    pub count: usize,
    /// What every code on the sheet becomes on first scan, or `None` for the chooser.
    pub handler: Option<&'a str>,
    pub note: Option<&'a str>,
    pub ec: EcLevel,
    pub id_len: usize,
    pub secret_len: usize,
}

/// A minted sheet, and the PDF of it.
pub struct Rendered {
    pub sheet_id: Uuid,
    pub pdf: Vec<u8>,
}

/// Mints the codes and renders them, in one transaction with the sheet row.
///
/// The PDF is rendered before the transaction commits, so a sheet that could not be
/// rendered never leaves codes behind that nobody printed. Nothing is written to the
/// blob store: the PDF carries every secret in plaintext, and the only copy meant to
/// outlive the request is the one on paper.
pub async fn mint_and_render(
    store: &Store,
    renderer: &SheetRenderer,
    order: Order<'_>,
) -> Result<Rendered, Error> {
    let mut tx = store.begin().await?;

    let handler_config = json!({});
    let sheet = store::insert_sheet(
        &mut *tx,
        store::NewSheet {
            id: Uuid::now_v7(),
            code_count: i32::try_from(order.count).unwrap_or(i32::MAX),
            base_url: order.base.as_str(),
            preset: order.preset,
            layout: &serde_json::to_value(order.layout).unwrap_or_else(|_| json!({})),
            handler_id: order.handler,
            handler_config: &handler_config,
            note: order.note,
        },
    )
    .await?;

    let minted = store::mint_codes(
        &mut tx,
        order.count,
        order.id_len,
        order.secret_len,
        Some(sheet.id),
        order.handler,
        &handler_config,
    )
    .await?;

    let request = SheetRequest {
        codes: minted
            .into_iter()
            .map(|MintedCode { id, secret }| (id, secret))
            .collect(),
        base: order.base.clone(),
        layout: order.layout.clone(),
        preset: order.preset.to_owned(),
        note: order.note.map(ToOwned::to_owned),
        ec: order.ec,
    };

    let pdf = renderer.render(&request).await?;

    store::record_event(
        &mut *tx,
        EventKind::SheetRendered,
        None,
        None,
        &json!({ "sheet": sheet.id, "count": order.count, "preset": order.preset }),
    )
    .await?;

    tx.commit().await.map_err(qr_core::Error::Database)?;

    Ok(Rendered {
        sheet_id: sheet.id,
        pdf,
    })
}

pub async fn run(args: SheetArgs, check_only: bool) -> Result<(), StartupError> {
    let base = BaseUrl::parse(&args.base_url).map_err(StartupError::BaseUrl)?;
    let ec = qr_sheet::parse_ec_level(&args.codes.qr_ec_level)?;
    let presets = load_presets(args.presets_file.as_deref())?;
    let layout = presets
        .get(&args.preset)?
        .offset_by(args.offset_x_mm, args.offset_y_mm);

    if check_only {
        return Ok(());
    }

    let renderer = SheetRenderer::new(
        args.typst.typst_bin.clone(),
        args.typst.typst_font_path.clone(),
    );

    let pdf = if args.calibration {
        renderer.render_calibration(&layout, &args.preset).await?
    } else {
        render_codes(&args, &base, &layout, ec, &renderer).await?
    };

    std::fs::write(&args.out, &pdf).map_err(|source| StartupError::Write {
        path: args.out.clone(),
        source,
    })?;

    println!("wrote {} ({} bytes)", args.out.display(), pdf.len());
    Ok(())
}

async fn render_codes(
    args: &SheetArgs,
    base: &BaseUrl,
    layout: &Layout,
    ec: EcLevel,
    renderer: &SheetRenderer,
) -> Result<Vec<u8>, StartupError> {
    let store = connect(args).await?;
    let rendered = mint_and_render(
        &store,
        renderer,
        Order {
            base,
            layout,
            preset: &args.preset,
            count: args.count.unwrap_or_else(|| layout.per_page()),
            handler: args.handler.as_deref(),
            note: args.note.as_deref(),
            ec,
            id_len: args.codes.code_id_len,
            secret_len: args.codes.code_secret_len,
        },
    )
    .await?;
    Ok(rendered.pdf)
}

async fn connect(args: &SheetArgs) -> Result<Store, StartupError> {
    let url = args.database.url()?;
    let pool = PgPoolOptions::new()
        .max_connections(args.database.database_max_connections)
        .connect(&url)
        .await
        .map_err(StartupError::Database)?;
    Ok(Store::new(pool))
}

/// The built-in layouts, or the ones in a file replacing them.
pub fn load_presets(path: Option<&Path>) -> Result<Presets, StartupError> {
    match path {
        None => Ok(Presets::builtin()?),
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|source| StartupError::Write {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(Presets::parse(&raw)?)
        }
    }
}

/// Prints what each combination of lengths and levels costs.
///
/// This is the tool for deciding those numbers against a ruler rather than against an
/// argument: print a sheet at each size, scan it from across a room, and read off which
/// row the one that worked came from.
pub fn budget(args: &BudgetArgs) -> Result<(), StartupError> {
    let base = BaseUrl::parse(&args.base_url).map_err(StartupError::BaseUrl)?;
    let levels = [
        qr_sheet::EcLevel::L,
        qr_sheet::EcLevel::M,
        qr_sheet::EcLevel::Q,
        qr_sheet::EcLevel::H,
    ];

    println!("{} at {} mm wide\n", base.as_str(), args.qr_mm);
    print!("{:>3} {:>7} {:>4}", "id", "secret", "url");
    for ec in levels {
        print!(
            "   {:>14}",
            format!(
                "{} ({})",
                qr_sheet::ec_level_name(ec),
                qr_sheet::ec_level_recovery(ec)
            )
        );
    }
    println!();

    for &id_len in &args.id_lens {
        for &secret_len in &args.secret_lens {
            let mut cells = Vec::with_capacity(levels.len());
            let mut url_len = 0;
            for ec in levels {
                let b = qr_sheet::budget(&base, id_len, secret_len, ec)?;
                url_len = b.url_len;
                cells.push(format!(
                    "v{} {}x{} {:.2}",
                    b.version,
                    b.modules,
                    b.modules,
                    b.mm_per_module(args.qr_mm)
                ));
            }
            print!("{id_len:>3} {secret_len:>7} {url_len:>4}");
            for cell in cells {
                print!("   {cell:>14}");
            }
            println!();
        }
    }

    println!(
        "\nmm/module is the printed module size; below about 0.6 a laser's toner spread \
         and a phone at arm's length start to disagree."
    );
    Ok(())
}

/// Reads the presets a deployment was given, so `--check-config` refuses a file that
/// does not parse rather than leaving it to the first print of the year.
pub fn check_presets(path: Option<&Path>) -> Result<(), StartupError> {
    load_presets(path).map(drop)
}
