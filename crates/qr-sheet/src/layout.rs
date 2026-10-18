//! Where the stickers sit on the page.
//!
//! Die-cut stock has no cut guides to aim at, so a layout is an origin and a pitch in
//! millimetres rather than a grid with gutters. Getting that wrong wastes a sheet of
//! labels, which is why the presets carry the stock's real geometry and a printer's own
//! offset is applied on top with [`Layout::offset_by`] instead of edited into one.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A4, which is the only paper this renders on.
pub const PAGE_W_MM: f32 = 210.0;
pub const PAGE_H_MM: f32 = 297.0;

/// A QR needs four empty modules on every side, or a scanner will not find its edges.
pub const QUIET_MODULES: f32 = 4.0;

/// How close to the paper's edge a consumer laser printer will put toner.
///
/// Stock that runs to the page edge, which two of the shipped presets do, will otherwise
/// have the outer columns clipped by the printer rather than by anything this code can
/// see.
pub const PRINTABLE_MARGIN_MM: f32 = 4.0;

/// The presets shipped with the binary.
const BUILTIN: &str = include_str!("../presets.toml");

/// The stock this prints on.
///
/// Named here rather than in the command line's default, so the one place that decides
/// it is beside the check that it is a preset still offered.
pub const DEFAULT_PRESET: &str = "relief-kopi-63";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default)]
    pub description: String,
    pub cols: u8,
    pub rows: u8,
    pub margin_left_mm: f32,
    pub margin_top_mm: f32,
    pub pitch_x_mm: f32,
    pub pitch_y_mm: f32,
    pub label_w_mm: f32,
    pub label_h_mm: f32,
    /// Side inset. Separate from the vertical one because stock that runs to the page
    /// edge needs clearance from the printer's unprintable margin on the sides, while
    /// giving the same away vertically would squeeze out the symbol.
    pub inset_x_mm: f32,
    pub inset_y_mm: f32,
    /// The symbol alone. The quiet zone is added around it.
    pub qr_mm: f32,
    pub label_lines: u8,
    pub cut_guides: bool,
    pub print_secret: bool,
    /// Still renders, but no longer offered.
    ///
    /// Every figure in these presets but one came off a manufacturer's datasheet rather
    /// than a caliper, and the one that was measured turned out not to match the
    /// datasheet it was taken from. A sheet of labels is what the difference costs, so
    /// the untested ones stay reachable by name for a link saved earlier and are kept
    /// out of everything that suggests stock to print on.
    #[serde(default)]
    pub deprecated: bool,
}

/// DejaVu Sans Mono's advance width, in ems. The caption is set in it, so this is what
/// turns a character count into a width without rendering anything.
const MONO_ADVANCE_EM: f32 = 0.6022;

/// A point in millimetres.
const PT_MM: f32 = 25.4 / 72.0;

/// Below this the address stops being readable at arm's length, so a layout that would
/// need smaller type is refused rather than printed.
const MIN_URL_PT: f32 = 5.0;

/// Above this the address competes with the symbol for attention, and on the big stock
/// it would take the whole box before the grid had any of it.
const MAX_URL_PT: f32 = 9.0;

const MIN_CELL_PT: f32 = 5.0;
const MAX_CELL_PT: f32 = 16.0;

/// The gutter between the symbol and the box beside it.
pub const CAPTION_GUTTER_MM: f32 = 1.5;

/// Clearance between the box's border and the address inside it.
pub const BOX_PAD_MM: f32 = 0.7;

/// How many characters a row of the typed grid holds.
///
/// Both are offered because the box is as wide as the stock leaves it: on a 63.5mm label
/// four across gives a cell about as tall as it is wide, and on a 99.1mm one it gives
/// four cells 15mm across holding a 6pt character.
const GRID_COLS: [usize; 2] = [4, 8];

/// A line of type occupies more than its point size: DejaVu's ascender and descender
/// together are a little over 1.1em and typst adds leading between lines. Rounded up
/// rather than derived, because the address is centred in the row it is given and only
/// an estimate that comes out too small can clip.
const LINE_EM: f32 = 1.45;

/// A cell holds one character, which needs its cap height and a little air.
const CELL_EM: f32 = 1.3;

/// How much of a cell's width one character may take before the grid reads as cramped.
const CELL_FILL: f32 = 0.72;

/// What is going into the box, in the terms the arithmetic needs it.
///
/// Lengths rather than the strings themselves: the id length is configurable and the
/// secret length with it, so none of the figures below can be assumed from the defaults.
#[derive(Debug, Clone, Copy)]
pub struct CaptionSpec {
    /// How many lines the address is printed on.
    pub url_lines: usize,
    /// The longest of those lines, in characters.
    pub longest_run: usize,
    /// Characters in the secret, or zero when this stock does not print one.
    pub secret_len: usize,
}

/// The box beside the symbol, in millimetres and points.
#[derive(Debug, Clone, Copy)]
pub struct Caption {
    pub box_w_mm: f32,
    pub box_h_mm: f32,
    pub url_pt: f32,
    /// What the address would have been set at unclamped, which is what says whether the
    /// box is too narrow rather than merely narrow.
    pub url_fit: f32,
    pub header_h_mm: f32,
    pub cols: usize,
    pub secret_rows: usize,
    /// Blank rows under the grid, from the stock's `label_lines`, to write on.
    pub blank_rows: usize,
    pub row_h_mm: f32,
    pub cell_pt: f32,
}

/// Width of `chars` monospaced characters at `pt`.
fn mono_width_mm(chars: usize, pt: f32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a caption is a few dozen characters at most"
    )]
    let chars = chars as f32;
    chars * MONO_ADVANCE_EM * pt * PT_MM
}

impl Layout {
    pub fn per_page(&self) -> usize {
        usize::from(self.cols) * usize::from(self.rows)
    }

    /// The same stock, moved by wherever this printer puts the page origin.
    ///
    /// A shift rather than a pair of replacement margins: the error belongs to the
    /// printer and is the same whichever stock goes through it, so it is measured once
    /// and applied to any preset.
    pub fn offset_by(&self, dx_mm: f32, dy_mm: f32) -> Self {
        Self {
            margin_left_mm: self.margin_left_mm + dx_mm,
            margin_top_mm: self.margin_top_mm + dy_mm,
            ..self.clone()
        }
    }

    /// The width the rendered SVG occupies, symbol plus quiet zone.
    pub fn qr_box_mm(&self, modules: usize) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a QR is at most 177 modules across, which an f32 holds exactly"
        )]
        let modules = modules as f32;
        self.qr_mm * (modules + 2.0 * QUIET_MODULES) / modules
    }

    pub fn mm_per_module(&self, modules: usize) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a QR is at most 177 modules across, which an f32 holds exactly"
        )]
        let modules = modules as f32;
        self.qr_mm / modules
    }

    /// What is left of the label once the inset and the symbol have taken their share.
    pub fn box_w_mm(&self, modules: usize) -> f32 {
        self.label_w_mm - 2.0 * self.inset_x_mm - self.qr_box_mm(modules) - CAPTION_GUTTER_MM
    }

    /// The box is drawn the height of the symbol rather than of the label.
    ///
    /// That is the whole point of it: the symbol is centred inside its quiet zone and the
    /// box is centred against the symbol, so the two borders land on the symbol's own top
    /// and bottom edges and the label's height is left to the symbol alone.
    pub fn box_h_mm(&self) -> f32 {
        self.qr_mm
    }

    pub fn caption_h_mm(&self) -> f32 {
        self.label_h_mm - 2.0 * self.inset_y_mm
    }

    /// Everything the box comes out as, worked out once.
    ///
    /// Both the fit check and the renderer read it from here, because the two disagreeing
    /// is a sheet that validates and then prints a grid with its last row off the bottom.
    pub fn caption(&self, modules: usize, spec: &CaptionSpec) -> Caption {
        let box_w_mm = self.box_w_mm(modules);
        let box_h_mm = self.box_h_mm();

        // Fitted to the longest run the address breaks into, not to the whole URL: the
        // runs are what is printed, one to a line.
        let unit = mono_width_mm(spec.longest_run.max(1), 1.0);
        let url_fit = (box_w_mm - 2.0 * BOX_PAD_MM).max(0.0) / unit;
        let url_pt = url_fit.clamp(MIN_URL_PT, MAX_URL_PT);

        #[expect(
            clippy::cast_precision_loss,
            reason = "an address breaks into a handful of runs"
        )]
        let url_lines = spec.url_lines.max(1) as f32;
        let header_h_mm = url_lines * url_pt * LINE_EM * PT_MM + 2.0 * BOX_PAD_MM;

        let blank_rows = usize::from(self.label_lines);
        let (cols, secret_rows) =
            self.grid_shape(box_w_mm, box_h_mm - header_h_mm, spec, blank_rows);

        #[expect(clippy::cast_precision_loss, reason = "a grid is a handful of rows")]
        let total_rows = (secret_rows + blank_rows) as f32;
        let (row_h_mm, cell_pt) = if total_rows == 0.0 {
            (0.0, MIN_CELL_PT)
        } else {
            let row_h_mm = (box_h_mm - header_h_mm) / total_rows;
            #[expect(clippy::cast_precision_loss, reason = "four or eight")]
            let cell_w_mm = box_w_mm / cols as f32;
            let by_height = row_h_mm / (CELL_EM * PT_MM);
            let by_width = cell_w_mm * CELL_FILL / (MONO_ADVANCE_EM * PT_MM);
            (
                row_h_mm,
                by_height.min(by_width).clamp(MIN_CELL_PT, MAX_CELL_PT),
            )
        };

        Caption {
            box_w_mm,
            box_h_mm,
            url_pt,
            url_fit,
            header_h_mm,
            cols,
            secret_rows,
            blank_rows,
            row_h_mm,
            cell_pt,
        }
    }

    /// How many characters to put on a row, and how many rows that makes.
    ///
    /// Whichever of the two widths leaves a cell closest to square. The box is as wide as
    /// the stock leaves it rather than as wide as the grid wants, so a single figure that
    /// suits one preset leaves another with cells five times wider than they are tall.
    fn grid_shape(
        &self,
        box_w_mm: f32,
        avail_h_mm: f32,
        spec: &CaptionSpec,
        blank_rows: usize,
    ) -> (usize, usize) {
        if spec.secret_len == 0 {
            return (GRID_COLS[0], 0);
        }
        let mut best = (
            GRID_COLS[0],
            spec.secret_len.div_ceil(GRID_COLS[0]),
            f32::MAX,
        );
        for cols in GRID_COLS {
            let rows = spec.secret_len.div_ceil(cols);
            #[expect(clippy::cast_precision_loss, reason = "a grid is a handful of rows")]
            let total = (rows + blank_rows) as f32;
            #[expect(clippy::cast_precision_loss, reason = "four or eight")]
            let cell_w = box_w_mm / cols as f32;
            if total <= 0.0 || avail_h_mm <= 0.0 {
                continue;
            }
            // Distance from square, measured in ratio rather than in millimetres so that
            // being twice too wide and twice too tall count the same.
            let aspect = (cell_w / (avail_h_mm / total))
                .max(f32::MIN_POSITIVE)
                .ln()
                .abs();
            if aspect < best.2 {
                best = (cols, rows, aspect);
            }
        }
        (best.0, best.1)
    }

    /// Refuses a layout that would print off the page, crowd the quiet zone out of a
    /// label, or clip the caption. All three produce a sheet that looks plausible on
    /// screen and wastes a sheet of stock.
    pub fn validate(&self, name: &str, modules: usize, spec: &CaptionSpec) -> Result<()> {
        let box_mm = self.qr_box_mm(modules);
        let quiet_mm = (box_mm - self.qr_mm) / 2.0;

        if box_mm > self.caption_h_mm() {
            return Err(Error::SymbolTooLarge {
                preset: name.to_owned(),
                qr_mm: self.qr_mm,
                quiet_mm,
                label_h_mm: self.label_h_mm,
            });
        }

        let caption = self.caption(modules, spec);

        if caption.box_w_mm <= 0.0 || caption.url_fit < MIN_URL_PT {
            return Err(Error::CaptionTooWide {
                preset: name.to_owned(),
                caption_w_mm: caption.box_w_mm,
                needed_pt: MIN_URL_PT,
            });
        }

        // The header is given what it needs and the rows divide what is left, so this
        // fires when the address alone has taken the box, or when the rows it has to
        // share the remainder with have shrunk the characters past reading.
        let rows = caption.secret_rows + caption.blank_rows;
        if caption.header_h_mm > caption.box_h_mm
            || (rows > 0
                && caption.cell_pt <= MIN_CELL_PT
                && caption.row_h_mm < MIN_CELL_PT * CELL_EM * PT_MM)
        {
            return Err(Error::CaptionTooTall {
                preset: name.to_owned(),
                caption_h_mm: caption.box_h_mm,
            });
        }

        let right = self.margin_left_mm
            + f32::from(self.cols.saturating_sub(1)) * self.pitch_x_mm
            + self.label_w_mm;
        if right > PAGE_W_MM {
            return Err(Error::OffPage {
                preset: name.to_owned(),
                edge: "the last column",
                side: "right",
            });
        }

        let bottom = self.margin_top_mm
            + f32::from(self.rows.saturating_sub(1)) * self.pitch_y_mm
            + self.label_h_mm;
        if bottom > PAGE_H_MM {
            return Err(Error::OffPage {
                preset: name.to_owned(),
                edge: "the last row",
                side: "bottom",
            });
        }

        // Where toner actually lands, which is inside the labels by the inset.
        for (edge, side, over) in [
            (
                "the first column",
                "left",
                self.margin_left_mm + self.inset_x_mm < PRINTABLE_MARGIN_MM,
            ),
            (
                "the last column",
                "right",
                right - self.inset_x_mm > PAGE_W_MM - PRINTABLE_MARGIN_MM,
            ),
            (
                "the first row",
                "top",
                self.margin_top_mm + self.inset_y_mm < PRINTABLE_MARGIN_MM,
            ),
            (
                "the last row",
                "bottom",
                bottom - self.inset_y_mm > PAGE_H_MM - PRINTABLE_MARGIN_MM,
            ),
        ] {
            if over {
                return Err(Error::OffPage {
                    preset: name.to_owned(),
                    edge,
                    side,
                });
            }
        }

        Ok(())
    }
}

/// Every layout this binary knows, by name.
#[derive(Debug, Clone)]
pub struct Presets(std::collections::BTreeMap<String, Layout>);

impl Presets {
    /// The presets compiled into the binary.
    pub fn builtin() -> Result<Self> {
        Self::parse(BUILTIN)
    }

    pub fn parse(toml_src: &str) -> Result<Self> {
        toml::from_str(toml_src)
            .map(Self)
            .map_err(Error::PresetSyntax)
    }

    pub fn get(&self, name: &str) -> Result<&Layout> {
        self.0
            .get(name)
            .ok_or_else(|| Error::UnknownPreset(name.to_owned()))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Layout)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The stock this is willing to recommend.
    ///
    /// What a listing shows and what an error names. [`iter`](Self::iter) still reaches
    /// the rest, because a deprecated preset renders exactly as it did before.
    pub fn current(&self) -> impl Iterator<Item = (&str, &Layout)> {
        self.iter().filter(|(_, layout)| !layout.deprecated)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    /// What a default-length redeem URL encodes to; pinned by crate::qr.
    const MODULES: usize = 33;

    /// The longest run the address breaks into: "qr.shine.town/c/", which beats the
    /// ten-character id on the line under it.
    const LONGEST: usize = 16;

    /// The default secret length, which is what fills the grid.
    const SECRET_LEN: usize = 16;

    fn presets() -> Presets {
        Presets::builtin().expect("builtin presets parse")
    }

    /// What the shipped defaults put in the box.
    fn spec(layout: &Layout) -> CaptionSpec {
        CaptionSpec {
            url_lines: 2,
            longest_run: LONGEST,
            secret_len: if layout.print_secret { SECRET_LEN } else { 0 },
        }
    }

    #[test]
    fn every_shipped_preset_parses_and_prints_within_its_stock() {
        let presets = presets();
        assert!(presets.names().count() >= 8);
        for (name, layout) in presets.iter() {
            layout
                .validate(name, MODULES, &spec(layout))
                .unwrap_or_else(|e| panic!("preset {name} does not fit: {e}"));
        }
    }

    /// The property the box exists for: its border meets the symbol's edges, so the
    /// label's height belongs to the symbol and the box takes none of it.
    #[test]
    fn the_box_is_drawn_the_height_of_the_symbol() {
        for (name, layout) in presets().iter() {
            let caption = layout.caption(MODULES, &spec(layout));
            assert!(
                (caption.box_h_mm - layout.qr_mm).abs() < f32::EPSILON,
                "{name} draws the box {}mm against a {}mm symbol",
                caption.box_h_mm,
                layout.qr_mm
            );
        }
    }

    /// Rows divide what the address leaves, so this is what says the grid is drawn
    /// inside the border rather than through it.
    #[test]
    fn the_rows_fill_the_box_and_no_more() {
        for (name, layout) in presets().iter() {
            let caption = layout.caption(MODULES, &spec(layout));
            #[expect(clippy::cast_precision_loss, reason = "a grid is a handful of rows")]
            let rows = (caption.secret_rows + caption.blank_rows) as f32;
            let total = caption.header_h_mm + rows * caption.row_h_mm;
            assert!(
                total <= caption.box_h_mm + 0.01,
                "{name} fills {total}mm of a {}mm box",
                caption.box_h_mm
            );
        }
    }

    /// Four across on narrow stock, eight on wide: a fixed figure leaves one preset with
    /// cells five times wider than they are tall.
    #[test]
    fn the_grid_is_as_wide_as_keeps_its_cells_nearest_square() {
        let presets = presets();
        let narrow = presets.get("relief-t7160").expect("t7160");
        assert_eq!(narrow.caption(MODULES, &spec(narrow)).cols, 4);

        let wide = presets.get("relief-t7163").expect("t7163");
        assert_eq!(wide.caption(MODULES, &spec(wide)).cols, 8);
    }

    #[test]
    fn stock_that_prints_no_secret_gets_no_grid() {
        let presets = presets();
        let small = presets.get("relief-kopi-48").expect("kopi-48");
        let caption = small.caption(MODULES, &spec(small));
        assert_eq!(caption.secret_rows, 0);
        assert_eq!(caption.blank_rows, 0);
    }

    /// The margins of this one are derived rather than read off a package, so the
    /// derivation is written down here: back to back in both directions, and what A4 has
    /// left over split evenly.
    #[test]
    fn the_back_to_back_stock_is_centred_on_the_page() {
        let presets = presets();
        let layout = presets.get("relief-kopi-63").expect("kopi-63");

        assert!((layout.pitch_x_mm - layout.label_w_mm).abs() < f32::EPSILON);
        assert!((layout.pitch_y_mm - layout.label_h_mm).abs() < f32::EPSILON);

        let across = f32::from(layout.cols) * layout.label_w_mm;
        let down = f32::from(layout.rows) * layout.label_h_mm;
        assert!((layout.margin_left_mm - (PAGE_W_MM - across) / 2.0).abs() < 0.01);
        assert!((layout.margin_top_mm - (PAGE_H_MM - down) / 2.0).abs() < 0.01);
    }

    #[test]
    fn a_printer_offset_moves_the_grid_and_leaves_the_stock_alone() {
        let presets = presets();
        let stock = presets.get("relief-kopi-63").expect("kopi-63");
        let shifted = stock.offset_by(-1.5, 0.8);

        assert!((shifted.margin_left_mm - (stock.margin_left_mm - 1.5)).abs() < 0.001);
        assert!((shifted.margin_top_mm - (stock.margin_top_mm + 0.8)).abs() < 0.001);
        assert!((shifted.pitch_x_mm - stock.pitch_x_mm).abs() < f32::EPSILON);
        assert!((shifted.label_h_mm - stock.label_h_mm).abs() < f32::EPSILON);
    }

    /// The offset is applied before the layout is checked rather than after, so a
    /// mistyped one is refused rather than printed with its last row off the paper.
    #[test]
    fn an_offset_that_pushes_the_grid_off_the_page_is_refused() {
        let presets = presets();
        let stock = presets.get("relief-kopi-63").expect("kopi-63");
        let shifted = stock.offset_by(0.0, 20.0);
        let err = shifted
            .validate("relief-kopi-63", MODULES, &spec(&shifted))
            .expect_err("should be refused");
        assert!(err.to_string().contains("bottom"), "wrong error: {err}");
    }

    /// The default has to be stock someone would be told to buy, and nothing checks
    /// that at startup: an unknown or deprecated name here is found at the printer.
    #[test]
    fn the_default_is_a_preset_that_is_still_offered() {
        let presets = presets();
        let layout = presets.get(DEFAULT_PRESET).expect("the default exists");
        assert!(!layout.deprecated, "{DEFAULT_PRESET} is deprecated");
    }

    /// Deprecated means unlisted, not removed: a link minted against one of these is on
    /// paper somewhere, and reprinting to match it has to stay possible.
    #[test]
    fn a_deprecated_preset_still_renders_and_is_simply_not_offered() {
        let presets = presets();
        let offered: Vec<_> = presets.current().map(|(name, _)| name).collect();
        assert_eq!(offered, vec![DEFAULT_PRESET]);

        let old = presets
            .get("relief-t7160")
            .expect("t7160 is still reachable");
        assert!(old.deprecated);
        old.validate("relief-t7160", MODULES, &spec(old))
            .expect("a deprecated preset still has to fit its stock");
    }

    #[test]
    fn every_shipped_preset_says_what_stock_it_is_for() {
        for (name, layout) in presets().iter() {
            assert!(!layout.description.is_empty(), "{name} has no description");
        }
    }

    #[test]
    fn the_quiet_zone_is_counted_outside_the_symbol() {
        let presets = presets();
        let plain = presets.get("plain").expect("plain");
        // Four modules each side of a 33-module symbol is 8/33 of its width again.
        let expected = plain.qr_mm * (33.0 + 8.0) / 33.0;
        assert!((plain.qr_box_mm(MODULES) - expected).abs() < 0.001);
    }

    #[test]
    fn the_marginal_preset_is_the_one_with_the_smallest_modules() {
        let smallest = presets()
            .iter()
            .min_by(|a, b| {
                a.1.mm_per_module(MODULES)
                    .total_cmp(&b.1.mm_per_module(MODULES))
            })
            .map(|(name, _)| name.to_owned());
        assert_eq!(smallest.as_deref(), Some("relief-kopi-48"));
    }

    #[test]
    fn no_shipped_preset_sets_type_below_the_legible_floor() {
        for (name, layout) in presets().iter() {
            let caption = layout.caption(MODULES, &spec(layout));
            assert!(
                caption.url_pt >= MIN_URL_PT && caption.url_pt <= MAX_URL_PT,
                "{name} would set the address at {}pt",
                caption.url_pt
            );
            assert!(
                caption.cell_pt >= MIN_CELL_PT,
                "{name} would set the grid at {}pt",
                caption.cell_pt
            );
        }
    }

    #[test]
    fn a_symbol_too_tall_for_its_label_is_refused() {
        let presets = presets();
        let mut layout = presets.get("relief-kopi-48").expect("preset").clone();
        layout.qr_mm = 30.0;
        assert!(
            layout
                .validate("overgrown", MODULES, &spec(&layout))
                .is_err()
        );
    }

    #[test]
    fn a_symbol_that_leaves_no_room_for_the_caption_is_refused() {
        // Portrait stock, which none of the shipped presets is: on a label wider than it
        // is tall the height runs out first, so this check only ever fires on stock
        // someone adds later.
        let layout = Layout {
            description: "portrait".to_owned(),
            cols: 1,
            rows: 1,
            margin_left_mm: 10.0,
            margin_top_mm: 10.0,
            pitch_x_mm: 40.0,
            pitch_y_mm: 80.0,
            label_w_mm: 40.0,
            label_h_mm: 80.0,
            inset_x_mm: 2.0,
            inset_y_mm: 2.0,
            qr_mm: 25.0,
            label_lines: 0,
            cut_guides: false,
            print_secret: false,
            deprecated: false,
        };
        let err = layout
            .validate("portrait", MODULES, &spec(&layout))
            .expect_err("should be refused");
        assert!(
            err.to_string().contains("beside the symbol"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn more_writing_lines_than_the_label_holds_are_refused() {
        let presets = presets();
        let mut layout = presets.get("relief-t7160").expect("preset").clone();
        layout.label_lines = 12;
        assert!(
            layout
                .validate("overfull", MODULES, &spec(&layout))
                .is_err()
        );
    }

    #[test]
    fn a_grid_wider_than_the_page_is_refused() {
        let presets = presets();
        let mut layout = presets.get("plain").expect("preset").clone();
        layout.cols = 9;
        assert!(
            layout
                .validate("overwide", MODULES, &spec(&layout))
                .is_err()
        );
    }

    #[test]
    fn content_inside_a_printers_unprintable_edge_is_refused() {
        // Stock that runs to the page edge is the case this exists for: the labels are
        // fine, but a printer will not put toner on the outer column's caption.
        let presets = presets();
        let mut layout = presets.get("relief-kopi-70").expect("preset").clone();
        layout.inset_x_mm = 0.5;
        let err = layout
            .validate("flush", MODULES, &spec(&layout))
            .expect_err("should be refused");
        assert!(err.to_string().contains("toner"), "wrong error: {err}");
    }

    #[test]
    fn the_zero_margin_presets_clear_that_edge_as_shipped() {
        let presets = presets();
        for name in ["relief-kopi-70", "relief-kopi-105"] {
            let layout = presets.get(name).expect(name);
            assert!(
                layout.margin_left_mm.abs() < f32::EPSILON,
                "{name} has a margin"
            );
            assert!(
                layout.inset_x_mm >= PRINTABLE_MARGIN_MM,
                "{name} relies on a margin it does not have"
            );
        }
    }

    #[test]
    fn an_unknown_preset_names_itself_in_the_error() {
        let err = presets().get("avery-l9999").expect_err("unknown");
        assert!(err.to_string().contains("avery-l9999"));
    }
}
