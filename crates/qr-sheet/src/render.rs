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
    qr_box_mm: f32,
    /// Where to type what is printed beside it, set on its side in its own column so
    /// the longest run of characters on the sticker costs width rather than type size.
    host: String,
    host_w_mm: f32,
    host_pt: f32,
    caption_pt: f32,
    secret_pt: f32,
    rule_gap_mm: f32,
    footer: String,
    footer_dy_mm: Option<f32>,
    codes: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    id: String,
    label: String,
    /// Broken across lines rather than set as one run: the whole secret on one line is
    /// the widest thing a caption would hold, and it is what pushes narrow stock over.
    secret_lines: Vec<String>,
    svg: String,
}

/// How many lines the printed secret is broken across.
const SECRET_LINES: usize = 2;

/// Splits a grouped secret into `SECRET_LINES` lines of whole groups.
fn secret_lines(secret: &Secret) -> Vec<String> {
    let grouped = secret.grouped();
    let groups: Vec<&str> = grouped.split(' ').collect();
    let per = groups.len().div_ceil(SECRET_LINES).max(1);
    groups.chunks(per).map(|chunk| chunk.join(" ")).collect()
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
            // already a bitmap by then: codes.json below carries the printed grouping,
            // never the URL.
            let svg_path = qr_dir.join(format!("{id}.svg"));
            write(&svg_path, symbol.svg.as_bytes())?;

            entries.push(Entry {
                id: id.to_string(),
                label: id.grouped(),
                secret_lines: secret_lines(secret),
                svg: format!("qr/{id}.svg"),
            });
        }

        let host = typed_host(&request.base);

        // The longest line the caption has to hold, measured off what was actually
        // generated rather than assumed from the default lengths, because the id length
        // is configurable and the type size is fitted to it. The host is not among them
        // any more: it is set on its side in a column of its own, so it costs width
        // rather than holding the type size of every other label down to its length.
        let printed_secret_lines = if request.layout.print_secret {
            SECRET_LINES
        } else {
            0
        };
        let longest = entries
            .iter()
            .flat_map(|e| {
                std::iter::once(e.label.chars().count())
                    .chain(e.secret_lines.iter().map(|l| l.chars().count()))
            })
            .max()
            .unwrap_or(1);

        // Only the secret stacks under the code now; the rule between them is counted
        // with the writing lines the layout already allows for.
        let caption_lines = printed_secret_lines;
        request
            .layout
            .validate(&request.preset, modules, longest, caption_lines)?;

        let document = Document {
            layout: &request.layout,
            preset: &request.preset,
            qr_box_mm: request.layout.qr_box_mm(modules),
            host: host.clone(),
            host_w_mm: request.layout.host_w_mm(),
            host_pt: request.layout.host_pt(),
            caption_pt: request.layout.caption_pt(modules, longest),
            secret_pt: request.layout.secret_pt(modules, longest),
            rule_gap_mm: crate::layout::RULE_GAP_MM,
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
            host: String::new(),
            host_w_mm: 0.0,
            host_pt: 7.0,
            caption_pt: 7.0,
            secret_pt: 6.0,
            rule_gap_mm: 0.0,
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
    fn a_secret_is_broken_into_whole_groups_across_two_lines() {
        let secret = Secret::parse("H7TQ4R9WX2ND5PKC").expect("secret");
        assert_eq!(secret_lines(&secret), vec!["H7TQ 4R9W", "X2ND 5PKC"]);
    }
}
