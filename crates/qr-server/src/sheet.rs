//! The sheet and budget subcommands.
//!
//! Printing is an administrative act that happens a handful of times a year, so the CLI
//! is the primary interface and the admin API renders through the same code rather than
//! beside it.

use std::path::Path;

use qr_core::{BaseUrl, MintedCode, Store, model::EventKind, store};
use qr_sheet::{Layout, Presets, SheetRenderer, SheetRequest};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;

use crate::{
    StartupError,
    cli::{BudgetArgs, SheetArgs},
};

pub async fn run(args: SheetArgs, check_only: bool) -> Result<(), StartupError> {
    let base = BaseUrl::parse(&args.base_url).map_err(StartupError::BaseUrl)?;
    let ec = qr_sheet::parse_ec_level(&args.codes.qr_ec_level)?;
    let presets = load_presets(args.presets_file.as_deref())?;
    let layout = presets.get(&args.preset)?.clone();

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

/// Mints the codes and renders them, in one transaction with the sheet row.
///
/// The PDF is rendered before the transaction commits and stored as a blob afterwards,
/// so a sheet that could not be rendered never leaves codes behind that nobody printed.
async fn render_codes(
    args: &SheetArgs,
    base: &BaseUrl,
    layout: &Layout,
    ec: qr_sheet::EcLevel,
    renderer: &SheetRenderer,
) -> Result<Vec<u8>, StartupError> {
    let count = args.count.unwrap_or_else(|| layout.per_page());
    let store = connect(args).await?;
    let mut tx = store.begin().await?;

    let sheet_id = uuid::Uuid::now_v7();
    let handler_config = json!({});
    let sheet = store::insert_sheet(
        &mut *tx,
        store::NewSheet {
            id: sheet_id,
            code_count: i32::try_from(count).unwrap_or(i32::MAX),
            base_url: base.as_str(),
            preset: &args.preset,
            layout: &serde_json::to_value(layout).unwrap_or_else(|_| json!({})),
            handler_id: args.handler.as_deref(),
            handler_config: &handler_config,
            note: args.note.as_deref(),
        },
    )
    .await?;

    let minted = store::mint_codes(
        &mut tx,
        count,
        args.codes.code_id_len,
        args.codes.code_secret_len,
        Some(sheet.id),
        args.handler.as_deref(),
        &handler_config,
    )
    .await?;

    let request = SheetRequest {
        codes: minted
            .iter()
            .map(|MintedCode { id, secret }| (*id, secret.clone()))
            .collect(),
        base: base.clone(),
        layout: layout.clone(),
        preset: args.preset.clone(),
        note: args.note.clone(),
        ec,
    };

    let pdf = renderer.render(&request).await?;

    store::record_event(
        &mut *tx,
        EventKind::SheetRendered,
        None,
        None,
        &json!({ "sheet": sheet.id, "count": count, "preset": args.preset }),
    )
    .await?;

    tx.commit().await.map_err(qr_core::Error::Database)?;
    Ok(pdf)
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

fn load_presets(path: Option<&Path>) -> Result<Presets, StartupError> {
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
