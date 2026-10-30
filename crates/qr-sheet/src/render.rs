//! Driving typst to produce the PDF.
//!
//! A subprocess rather than the typst crate: using it as a library means implementing its
//! `World` trait — file resolution, the font book, package resolution, a clock — and
//! carrying a couple of hundred crates that have to be followed through every typst
//! release, for a feature that runs a handful of times a year.

use std::{path::PathBuf, process::Stdio, time::Duration};

use qr_core::{BaseUrl, CodeId, Secret};
use qrcode::EcLevel;
use serde::Serialize;
use tokio::process::Command;

use crate::{Error, Layout, Result, qr};

const SHEET_TYPST: &str = include_str!("../typst/sheet.typ");
const CALIBRATION_TYPST: &str = include_str!("../typst/calibration.typ");

/// A4's height, past which a footer would overprint the last row.
const PAGE_H_MM: f32 = crate::layout::PAGE_H_MM;

/// What to put on a sheet.
pub struct SheetRequest {
    pub codes: Vec<(CodeId, Secret)>,
    pub base: BaseUrl,
    pub layout: Layout,
    pub preset: String,
    pub note: Option<String>,
    pub ec: EcLevel,
}

/// The document handed to typst. Field names are what sheet.typ reads.
#[derive(Serialize)]
struct Document<'a> {
    layout: &'a Layout,
    preset: &'a str,
    /// The host printed above every symbol, without the scheme.
    host: String,
    host_pt: f32,
    host_gap_mm: f32,
    /// The left half of the label, which holds the host and the symbol.
    half_w_mm: f32,
    qr_box_mm: f32,
    footer: String,
    footer_dy_mm: Option<f32>,
    codes: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    svg: String,
}

pub struct SheetRenderer {
    typst_bin: PathBuf,
    font_path: Option<PathBuf>,
    timeout: Duration,
}

impl SheetRenderer {
    pub fn new(typst_bin: PathBuf, font_path: Option<PathBuf>) -> Self {
        Self {
            typst_bin,
            font_path,
            timeout: Duration::from_secs(60),
        }
    }

    /// Renders the stickers and returns the PDF.
    pub async fn render(&self, request: &SheetRequest) -> Result<Vec<u8>> {
        let dir = tempfile::tempdir().map_err(|source| Error::Io {
            action: "create a working directory for",
            path: PathBuf::from("the sheet"),
            source,
        })?;
        let root = dir.path();
        let qr_dir = root.join("qr");
        std::fs::create_dir(&qr_dir).map_err(|source| Error::Io {
            action: "create",
            path: qr_dir.clone(),
            source,
        })?;

        let mut entries = Vec::with_capacity(request.codes.len());
        let mut modules = 0usize;
        let mut version = 0i16;

        for (id, secret) in &request.codes {
            let url = request.base.redeem_url(id, secret);
            let symbol = qr::encode(id.as_str(), &url, request.ec)?;
            modules = symbol.modules;
            version = symbol.version;

            // The SVG is the only place the plaintext secret lands on disk, and it is
            // already a bitmap by then: nothing printed beside the symbol carries the id,
            // the secret or the URL that joins them.
            let svg_path = qr_dir.join(format!("{id}.svg"));
            write(&svg_path, symbol.svg.as_bytes())?;

            entries.push(Entry {
                svg: format!("qr/{id}.svg"),
            });
        }

        let host = typed_host(&request.base);
        let host_chars = host.chars().count();

        request
            .layout
            .validate(&request.preset, modules, host_chars)?;
        let label = request.layout.label(modules, host_chars);

        let document = Document {
            layout: &request.layout,
            preset: &request.preset,
            host,
            host_pt: label.host_pt,
            host_gap_mm: label.gap_mm,
            half_w_mm: label.half_w_mm,
            qr_box_mm: label.qr_box_mm,
            footer: footer(request, version, modules),
            footer_dy_mm: footer_dy_mm(&request.layout),
            codes: entries,
        };

        self.compile(root, SHEET_TYPST, &document).await
    }

    /// Renders crosshairs where this layout believes the labels are.
    pub async fn render_calibration(&self, layout: &Layout, preset: &str) -> Result<Vec<u8>> {
        let dir = tempfile::tempdir().map_err(|source| Error::Io {
            action: "create a working directory for",
            path: PathBuf::from("the calibration sheet"),
            source,
        })?;

        let document = Document {
            layout,
            preset,
            host: String::new(),
            host_pt: 7.0,
            host_gap_mm: 0.0,
            half_w_mm: 0.0,
            qr_box_mm: 0.0,
            footer: String::new(),
            footer_dy_mm: None,
            codes: Vec::new(),
        };

        self.compile(dir.path(), CALIBRATION_TYPST, &document).await
    }

    /// Writes the template and its data into `root` and compiles it there.
    ///
    /// `--root` at the temporary directory is the sandbox: whatever the template says, it
    /// can read nothing outside the files written here.
    async fn compile(
        &self,
        root: &std::path::Path,
        template: &str,
        document: &Document<'_>,
    ) -> Result<Vec<u8>> {
        let main = root.join("main.typ");
        write(&main, template.as_bytes())?;

        let json = serde_json::to_vec_pretty(document).map_err(|e| {
            Error::Core(qr_core::Error::Invalid(format!(
                "the sheet document would not serialise: {e}"
            )))
        })?;
        write(&root.join("codes.json"), &json)?;

        let out = root.join("out.pdf");

        let mut command = Command::new(&self.typst_bin);
        command
            .arg("compile")
            .arg("--root")
            .arg(root)
            // Reproducibility, not tidiness: this machine has a few hundred fonts and a
            // container has none, and a template that silently picks one up here would
            // fall back to something else there.
            .arg("--ignore-system-fonts");
        if let Some(fonts) = &self.font_path {
            command.arg("--font-path").arg(fonts);
        }
        command
            .arg(&main)
            .arg(&out)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let run = command.output();
        let output = match tokio::time::timeout(self.timeout, run).await {
            Err(_) => return Err(Error::TypstTimeout(self.timeout.as_secs())),
            Ok(Err(source)) => return Err(Error::TypstMissing(source)),
            Ok(Ok(output)) => output,
        };

        if !output.status.success() {
            return Err(Error::TypstFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }

        std::fs::read(&out).map_err(|source| Error::Io {
            action: "read back",
            path: out,
            source,
        })
    }
}

/// The host printed above the symbol, without the scheme.
///
/// Lowercase and bare: it is read by a human rather than encoded, nothing about `https`
/// helps them, and every character saved is type size kept on a narrow label.
fn typed_host(base: &BaseUrl) -> String {
    base.as_str()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

fn write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|source| Error::Io {
        action: "write",
        path: path.to_path_buf(),
        source,
    })
}

/// Provenance, printed small at the foot.
///
/// The numbers are here because a sheet outlives the command that made it: holding two
/// printings side by side is how the measuring session tells which settings scanned.
fn footer(request: &SheetRequest, version: i16, modules: usize) -> String {
    let note = request.note.as_deref().unwrap_or("");
    format!(
        "{note}{sep}{preset} · {count} codes · QR v{version} {modules}×{modules} \
         ec{ec} · {mm:.2} mm/module",
        sep = if note.is_empty() { "" } else { " · " },
        preset = request.preset,
        count = request.codes.len(),
        ec = qr::ec_level_name(request.ec),
        mm = request.layout.mm_per_module(modules),
    )
}

/// Where the footer goes, or `None` when the last row already reaches the foot.
fn footer_dy_mm(layout: &Layout) -> Option<f32> {
    let bottom = layout.margin_top_mm
        + f32::from(layout.rows.saturating_sub(1)) * layout.pitch_y_mm
        + layout.label_h_mm;
    let dy = bottom + 2.0;
    (dy + 4.0 <= PAGE_H_MM).then_some(dy)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::Presets;

    fn layout(name: &str) -> Layout {
        Presets::builtin()
            .expect("presets")
            .get(name)
            .expect(name)
            .clone()
    }

    #[test]
    fn a_footer_is_dropped_when_the_last_row_reaches_the_foot_of_the_page() {
        // 24 up with no side margin ends at 292.5mm, which leaves no room.
        assert_eq!(footer_dy_mm(&layout("relief-kopi-70")), None);
        // The full-sheet label leaves none either.
        assert_eq!(footer_dy_mm(&layout("relief-t7167")), None);
        assert!(footer_dy_mm(&layout("relief-t7160")).is_some());
    }

    /// The scheme is left off: it is read by a human rather than encoded, and every
    /// browser puts it back.
    #[test]
    fn the_printed_host_drops_the_scheme_and_trailing_slash() {
        let base = BaseUrl::parse("https://qr.shine.town/").expect("base");
        assert_eq!(typed_host(&base), "qr.shine.town");
    }
}
