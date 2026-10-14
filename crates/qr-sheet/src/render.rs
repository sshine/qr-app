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

use crate::{Error, Layout, Result, layout::CaptionSpec, qr};

const SHEET_TYPST: &str = include_str!("../typst/sheet.typ");
const CALIBRATION_TYPST: &str = include_str!("../typst/calibration.typ");

/// How many lines the address is printed on.
const URL_LINES: usize = 2;

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
    qr_box_mm: f32,
    /// The box beside the symbol, drawn the symbol's own height.
    box_w_mm: f32,
    box_h_mm: f32,
    box_pad_mm: f32,
    /// The row the address sits in, and the height the rows under it share.
    header_h_mm: f32,
    row_h_mm: f32,
    cols: usize,
    /// Rows left blank under the grid, to write on.
    blank_rows: usize,
    url_pt: f32,
    cell_pt: f32,
    footer: String,
    footer_dy_mm: Option<f32>,
    codes: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    id: String,
    /// The address broken after the path, so the longest run printed is the host and
    /// the code gets a line to itself.
    url_lines: Vec<String>,
    /// The secret, one character to a cell, in reading order.
    cells: Vec<String>,
    svg: String,
}

/// The address as it is printed: where to type, then what to type there.
///
/// The scheme is left off. Every browser and every phone keyboard puts it back, and the
/// run it would add is longer than the host, so printing it would set the whole address
/// smaller on exactly the stock that can least afford it.
fn url_lines(base: &BaseUrl, id: &CodeId) -> Vec<String> {
    vec![format!("{}/c/", typed_host(base)), id.to_string()]
}

/// The secret split into the cells it is printed in, padded to fill its last row.
///
/// Ungrouped: a cell is the grouping. The spaces a printed secret carries exist to keep
/// a reader's place in a run of sixteen characters, and a grid of boxes does that better
/// than a space does.
fn cells(secret: &Secret, cols: usize) -> Vec<String> {
    let mut cells: Vec<String> = secret.expose().chars().map(|c| c.to_string()).collect();
    while cells.len() % cols != 0 {
        cells.push(String::new());
    }
    cells
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
            // already a bitmap by then: codes.json below carries the printed characters,
            // never the URL that joins them.
            let svg_path = qr_dir.join(format!("{id}.svg"));
            write(&svg_path, symbol.svg.as_bytes())?;

            entries.push(Entry {
                id: id.to_string(),
                url_lines: url_lines(&request.base, id),
                cells: Vec::new(),
                svg: format!("qr/{id}.svg"),
            });
        }

        // Measured off what was actually generated rather than assumed from the default
        // lengths, because both the id length and the secret length are configurable and
        // the box is fitted to them.
        let longest_run = entries
            .iter()
            .flat_map(|e| e.url_lines.iter().map(|l| l.chars().count()))
            .max()
            .unwrap_or(1);
        let secret_len = if request.layout.print_secret {
            request
                .codes
                .first()
                .map_or(0, |(_, secret)| secret.expose().chars().count())
        } else {
            0
        };
        let spec = CaptionSpec {
            url_lines: URL_LINES,
            longest_run,
            secret_len,
        };

        request.layout.validate(&request.preset, modules, &spec)?;
        let caption = request.layout.caption(modules, &spec);

        if secret_len > 0 {
            for (entry, (_, secret)) in entries.iter_mut().zip(&request.codes) {
                entry.cells = cells(secret, caption.cols);
            }
        }

        // With nothing under it the address is given the whole box, rather than left
        // floating above an empty half of one.
        let rows = caption.secret_rows + caption.blank_rows;
        let header_h_mm = if rows == 0 {
            caption.box_h_mm
        } else {
            caption.header_h_mm
        };

        let document = Document {
            layout: &request.layout,
            preset: &request.preset,
            qr_box_mm: request.layout.qr_box_mm(modules),
            box_w_mm: caption.box_w_mm,
            box_h_mm: caption.box_h_mm,
            box_pad_mm: crate::layout::BOX_PAD_MM,
            header_h_mm,
            row_h_mm: caption.row_h_mm,
            cols: caption.cols,
            blank_rows: caption.blank_rows,
            url_pt: caption.url_pt,
            cell_pt: caption.cell_pt,
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
            qr_box_mm: 0.0,
            box_w_mm: 0.0,
            box_h_mm: 0.0,
            box_pad_mm: 0.0,
            header_h_mm: 0.0,
            row_h_mm: 0.0,
            cols: 0,
            blank_rows: 0,
            url_pt: 7.0,
            cell_pt: 7.0,
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

/// The host a person types when the QR will not scan, without the scheme.
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

    #[test]
    fn a_secret_is_printed_one_character_to_a_cell() {
        let secret = Secret::parse("H7TQ4R9WX2ND5PKC").expect("secret");
        let cells = cells(&secret, 4);
        assert_eq!(cells.len(), 16);
        assert_eq!(cells.first().map(String::as_str), Some("H"));
        assert_eq!(cells.last().map(String::as_str), Some("C"));
    }

    /// The grid is drawn row by row, so a secret that does not divide by the row width
    /// would otherwise leave typst a row short of cells.
    #[test]
    fn a_secret_that_does_not_fill_its_last_row_is_padded_out() {
        let secret = Secret::parse("H7TQ4R9WX2ND5P").expect("secret");
        let cells = cells(&secret, 4);
        assert_eq!(cells.len(), 16);
        assert_eq!(cells.last().map(String::as_str), Some(""));
    }

    /// The scheme is left off, and the break is after the path rather than inside the id.
    #[test]
    fn the_address_breaks_after_the_path() {
        let base = BaseUrl::parse("https://qr.shine.town").expect("base");
        let id = CodeId::parse("RDTAPG55XY").expect("id");
        assert_eq!(
            url_lines(&base, &id),
            vec!["qr.shine.town/c/", "RDTAPG55XY"]
        );
    }
}
