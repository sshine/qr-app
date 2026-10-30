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

/// The quiet zone the QR standard asks for: four empty modules on every side, or a scanner
/// may not find the symbol's edges.
pub const QUIET_MODULES: u8 = 4;

fn standard_quiet_modules() -> u8 {
    QUIET_MODULES
}

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
    /// The largest symbol the stock may carry. The renderer uses the largest that fits
    /// the left half of the label beside the printed host.
    pub qr_mm: f32,
    /// Empty modules on each side of the symbol.
    ///
    /// Below the standard's four only on stock whose rows sit back to back, where the
    /// border the standard asks for is most of the white between two rows of codes; a
    /// phone camera finds a symbol with two.
    #[serde(default = "standard_quiet_modules")]
    pub quiet_modules: u8,
    pub cut_guides: bool,
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

/// DejaVu Sans Mono's advance width, in ems. The host is set in it, so this is what turns
/// a character count into a width without rendering anything.
const MONO_ADVANCE_EM: f32 = 0.6022;

/// A point in millimetres.
const PT_MM: f32 = 25.4 / 72.0;

/// Below this the host stops being readable at arm's length, so a layout that would need
/// smaller type is refused rather than printed.
const MIN_HOST_PT: f32 = 5.0;

/// Above this the host competes with the symbol for attention.
const MAX_HOST_PT: f32 = 10.0;

/// Clearance between the host and the edges of its half-label.
const HOST_PAD_MM: f32 = 0.7;

/// The white space between the host and the symbol beneath it.
pub const HOST_GAP_MM: f32 = 1.0;

/// A line of type occupies more than its point size: DejaVu's ascender and descender
/// together are a little over 1.1em and typst adds leading. Rounded up rather than
/// derived, because being a little too small only shifts the symbol down.
const HOST_LINE_EM: f32 = 1.25;

/// A symbol smaller than this is not worth putting on stock; the smallest shipped
/// preset still clears it.
const MIN_QR_BOX_MM: f32 = 15.0;

/// Half the label's usable area, with the host above the symbol. Worked out once so the
/// fit check and the renderer cannot disagree about how much room is left.
#[derive(Debug, Clone, Copy)]
pub struct Label {
    pub content_w_mm: f32,
    pub content_h_mm: f32,
    /// The left half, which holds the host and the symbol. The right half is blank.
    pub half_w_mm: f32,
    /// What the host would be set at unclamped, which is what says the half is too narrow
    /// rather than merely narrow.
    pub host_fit_pt: f32,
    pub host_pt: f32,
    pub host_h_mm: f32,
    pub gap_mm: f32,
    /// The symbol and its quiet zone, scaled down to fit the half-label.
    pub qr_box_mm: f32,
}

/// Width of `chars` monospaced characters at `pt`.
fn mono_width_mm(chars: usize, pt: f32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a host is a few dozen characters at most"
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

    /// The width the rendered SVG occupies, symbol plus quiet zone, before any scaling
    /// to fit the half-label.
    pub fn qr_box_mm(&self, modules: usize) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a QR is at most 177 modules across, which an f32 holds exactly"
        )]
        let modules = modules as f32;
        self.qr_mm * (modules + 2.0 * f32::from(self.quiet_modules)) / modules
    }

    pub fn mm_per_module(&self, modules: usize) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a QR is at most 177 modules across, which an f32 holds exactly"
        )]
        let modules = modules as f32;
        self.qr_mm / modules
    }

    pub fn content_w_mm(&self) -> f32 {
        self.label_w_mm - 2.0 * self.inset_x_mm
    }

    pub fn content_h_mm(&self) -> f32 {
        self.label_h_mm - 2.0 * self.inset_y_mm
    }

    /// The left half of the label, worked out from the stock and the host it has to print.
    ///
    /// The host is set as large as the half-label allows, and the symbol is then given the
    /// largest box that still fits under it. Capping rather than refusing is deliberate:
    /// the preset's `qr_mm` is the size the stock was chosen for, and a host line makes
    /// that a little smaller on every stock rather than a reason to stop printing.
    pub fn label(&self, modules: usize, host_chars: usize) -> Label {
        let content_w_mm = self.content_w_mm();
        let content_h_mm = self.content_h_mm();
        let half_w_mm = content_w_mm / 2.0;

        let unit = mono_width_mm(host_chars.max(1), 1.0);
        let host_fit = (half_w_mm - 2.0 * HOST_PAD_MM).max(0.0) / unit;
        let host_pt = host_fit.clamp(MIN_HOST_PT, MAX_HOST_PT);
        let host_h_mm = host_pt * HOST_LINE_EM * PT_MM;

        let available_h_mm = (content_h_mm - host_h_mm - HOST_GAP_MM).max(0.0);
        let qr_box_mm = self.qr_box_mm(modules).min(half_w_mm).min(available_h_mm);

        Label {
            content_w_mm,
            content_h_mm,
            half_w_mm,
            host_fit_pt: host_fit,
            host_pt,
            host_h_mm,
            gap_mm: HOST_GAP_MM,
            qr_box_mm,
        }
    }

    /// Whether this stock can hold the host and a scannable symbol in its left half.
    pub fn fits(&self, host_chars: usize, modules: usize) -> bool {
        let label = self.label(modules, host_chars);
        label.half_w_mm > 0.0
            && label.host_fit_pt >= MIN_HOST_PT
            && label.qr_box_mm >= MIN_QR_BOX_MM
            && label.host_h_mm + label.gap_mm + label.qr_box_mm <= label.content_h_mm + 0.001
    }

    /// Refuses a layout that would print off the page, or that leaves the left half too
    /// small for the host and a scannable symbol. Both produce a sheet that looks
    /// plausible on screen and wastes a sheet of stock.
    pub fn validate(&self, name: &str, modules: usize, host_chars: usize) -> Result<()> {
        let label = self.label(modules, host_chars);

        if label.half_w_mm <= 0.0 || label.host_fit_pt < MIN_HOST_PT {
            return Err(Error::HostTooWide {
                preset: name.to_owned(),
                half_w_mm: label.half_w_mm,
                needed_pt: MIN_HOST_PT,
            });
        }

        if label.qr_box_mm < MIN_QR_BOX_MM {
            return Err(Error::SymbolTooSmall {
                preset: name.to_owned(),
                qr_box_mm: label.qr_box_mm,
                half_w_mm: label.half_w_mm,
                content_h_mm: label.content_h_mm,
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

    /// The host the default base URL prints: "qr.shine.town".
    const HOST_CHARS: usize = 12;

    fn presets() -> Presets {
        Presets::builtin().expect("builtin presets parse")
    }

    #[test]
    fn every_shipped_preset_parses_and_prints_within_its_stock() {
        let presets = presets();
        assert!(presets.names().count() >= 8);
        for (name, layout) in presets.iter() {
            layout
                .validate(name, MODULES, HOST_CHARS)
                .unwrap_or_else(|e| panic!("preset {name} does not fit: {e}"));
        }
    }

    /// The host and the symbol share one half of the label. The right half is the part
    /// left for writing, so neither may spill into it.
    #[test]
    fn the_host_and_symbol_fill_the_left_half_and_no_more() {
        for (name, layout) in presets().iter() {
            let label = layout.label(MODULES, HOST_CHARS);
            assert!(
                label.qr_box_mm <= label.half_w_mm + 0.001,
                "{name} draws a {}mm symbol in a {}mm half",
                label.qr_box_mm,
                label.half_w_mm
            );
            assert!(
                label.host_h_mm + label.gap_mm + label.qr_box_mm <= label.content_h_mm + 0.001,
                "{name} stacks more than the {}mm label holds",
                label.content_h_mm
            );
            assert!(
                (label.half_w_mm * 2.0 - label.content_w_mm).abs() < 0.001,
                "{name} does not split the content width in two"
            );
        }
    }

    /// Above the symbol means above it: the host has a height of its own and the symbol
    /// starts under the gap, not beside the text.
    #[test]
    fn the_host_has_a_line_of_its_own_above_the_symbol() {
        for (name, layout) in presets().iter() {
            let label = layout.label(MODULES, HOST_CHARS);
            assert!(label.host_h_mm > 0.0, "{name} gives the host no height");
            assert!(label.gap_mm > 0.0, "{name} leaves no gap under the host");
        }
    }

    #[test]
    fn no_shipped_preset_sets_the_host_below_the_legible_floor() {
        for (name, layout) in presets().iter() {
            let label = layout.label(MODULES, HOST_CHARS);
            assert!(
                label.host_pt >= MIN_HOST_PT && label.host_pt <= MAX_HOST_PT,
                "{name} would set the host at {}pt",
                label.host_pt
            );
        }
    }

    /// The symbol is capped by the half-label rather than by the preset's qr_mm alone,
    /// and no preset is allowed to keep a symbol that would reach the blank half.
    #[test]
    fn the_symbol_is_capped_by_its_half_and_its_height() {
        for (name, layout) in presets().iter() {
            let label = layout.label(MODULES, HOST_CHARS);
            let available = (label.content_h_mm - label.host_h_mm - label.gap_mm).max(0.0);
            let expected = layout
                .qr_box_mm(MODULES)
                .min(label.half_w_mm)
                .min(available);
            assert!(
                (label.qr_box_mm - expected).abs() < 0.001,
                "{name} did not take the smallest of symbol, half-width and height"
            );
        }

        let narrow = presets().get("relief-kopi-63").expect("kopi-63").clone();
        assert!(
            narrow.label(MODULES, HOST_CHARS).qr_box_mm < narrow.qr_box_mm(MODULES),
            "a symbol wider than the half-label was not scaled down"
        );
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
            .validate("relief-kopi-63", MODULES, HOST_CHARS)
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
        old.validate("relief-t7160", MODULES, HOST_CHARS)
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
    fn a_host_too_long_for_its_half_is_refused() {
        let presets = presets();
        let mut layout = presets.get("relief-kopi-63").expect("preset").clone();
        layout.label_w_mm = 20.0;
        let err = layout
            .validate("narrow", MODULES, HOST_CHARS)
            .expect_err("should be refused");
        assert!(err.to_string().contains("host"), "wrong error: {err}");
    }

    #[test]
    fn a_symbol_with_no_room_left_for_the_host_is_refused() {
        let presets = presets();
        let mut layout = presets.get("relief-kopi-63").expect("preset").clone();
        layout.qr_mm = 1.0;
        let err = layout
            .validate("tiny", MODULES, HOST_CHARS)
            .expect_err("should be refused");
        assert!(err.to_string().contains("symbol"), "wrong error: {err}");
    }

    #[test]
    fn a_grid_wider_than_the_page_is_refused() {
        let presets = presets();
        let mut layout = presets.get("plain").expect("preset").clone();
        layout.cols = 9;
        assert!(layout.validate("overwide", MODULES, HOST_CHARS).is_err());
    }

    #[test]
    fn content_inside_a_printers_unprintable_edge_is_refused() {
        // Stock that runs to the page edge is the case this exists for: the labels are
        // fine, but a printer will not put toner on the outer column.
        let presets = presets();
        let mut layout = presets.get("relief-kopi-70").expect("preset").clone();
        layout.inset_x_mm = 0.5;
        let err = layout
            .validate("flush", MODULES, HOST_CHARS)
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
