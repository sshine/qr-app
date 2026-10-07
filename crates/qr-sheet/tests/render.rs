//! Rendering a real PDF through a real typst.
//!
//! Ignored rather than feature-gated, so `--all-features` cannot switch these on in an
//! environment with no typst. `nix flake check` and `just test-db` run them with
//! `--include-ignored`; QR_TYPST_BIN points at the one the flake pins.

#![allow(clippy::expect_used)]

use qr_core::{BaseUrl, CodeId, Secret, ids};
use qr_sheet::{EcLevel, Presets, SheetRenderer, SheetRequest};

fn renderer() -> SheetRenderer {
    let bin = std::env::var("QR_TYPST_BIN").unwrap_or_else(|_| "typst".to_owned());
    SheetRenderer::new(bin.into(), None)
}

fn codes(n: usize) -> Vec<(CodeId, Secret)> {
    (0..n)
        .map(|_| {
            (
                CodeId::generate(ids::DEFAULT_CODE_ID_LEN).expect("id"),
                Secret::generate(ids::DEFAULT_SECRET_LEN).expect("secret"),
            )
        })
        .collect()
}

fn request(preset: &str, count: usize) -> SheetRequest {
    let presets = Presets::builtin().expect("presets");
    SheetRequest {
        codes: codes(count),
        base: BaseUrl::parse("https://qr.shine.town").expect("base"),
        layout: presets.get(preset).expect(preset).clone(),
        preset: preset.to_owned(),
        note: Some("garage, 2026".to_owned()),
        ec: EcLevel::Q,
    }
}

/// Writes a rendered PDF out when QR_SHEET_DUMP_DIR is set.
///
/// Nothing asserts on the result; it is for holding a sheet against real label stock,
/// which is the only way some of these numbers can be checked.
fn dump(name: &str, pdf: &[u8]) {
    if let Ok(dir) = std::env::var("QR_SHEET_DUMP_DIR") {
        let path = std::path::Path::new(&dir).join(format!("{name}.pdf"));
        std::fs::write(&path, pdf).expect("dump");
        eprintln!("wrote {}", path.display());
    }
}

/// Counts page objects, which is the only structural thing worth asserting without
/// parsing PDF properly.
fn pages(pdf: &[u8]) -> usize {
    pdf.windows(10)
        .filter(|w| w == b"/Type /Pag" || w == b"/Type/Page")
        .count()
}

#[tokio::test]
#[ignore = "needs typst"]
async fn every_shipped_preset_renders_a_full_sheet() {
    let presets = Presets::builtin().expect("presets");
    let renderer = renderer();

    for (name, layout) in presets.iter() {
        let pdf = renderer
            .render(&request(name, layout.per_page()))
            .await
            .unwrap_or_else(|e| panic!("{name} did not render: {e}"));

        assert!(pdf.starts_with(b"%PDF"), "{name} produced no PDF");
        assert!(pdf.len() > 2000, "{name} produced {} bytes", pdf.len());
        dump(name, &pdf);
    }
}

#[tokio::test]
#[ignore = "needs typst"]
async fn more_codes_than_fit_spill_onto_further_pages() {
    let renderer = renderer();
    let presets = Presets::builtin().expect("presets");
    let per_page = presets.get("relief-t7160").expect("preset").per_page();

    let one = renderer
        .render(&request("relief-t7160", per_page))
        .await
        .expect("one page");
    let three = renderer
        .render(&request("relief-t7160", per_page * 2 + 1))
        .await
        .expect("three pages");

    assert!(pages(&one) >= 1);
    assert!(
        pages(&three) > pages(&one),
        "a sheet and a bit should be more pages than a sheet"
    );
}

#[tokio::test]
#[ignore = "needs typst"]
async fn a_partly_filled_sheet_renders() {
    let pdf = renderer()
        .render(&request("relief-t7160", 4))
        .await
        .expect("render");
    assert!(pdf.starts_with(b"%PDF"));
}

#[tokio::test]
#[ignore = "needs typst"]
async fn every_shipped_preset_renders_a_calibration_sheet() {
    let presets = Presets::builtin().expect("presets");
    let renderer = renderer();

    for (name, layout) in presets.iter() {
        let pdf = renderer
            .render_calibration(layout, name)
            .await
            .unwrap_or_else(|e| panic!("{name} did not calibrate: {e}"));
        assert!(pdf.starts_with(b"%PDF"), "{name} produced no PDF");
        dump(&format!("{name}-calibration"), &pdf);
    }
}

#[tokio::test]
#[ignore = "needs typst"]
async fn a_layout_the_stock_cannot_hold_is_refused_before_typst_is_reached() {
    let mut request = request("relief-kopi-48", 1);
    request.layout.qr_mm = 40.0;
    let err = renderer()
        .render(&request)
        .await
        .expect_err("should be refused");
    assert!(err.to_string().contains("quiet zone"), "wrong error: {err}");
}
