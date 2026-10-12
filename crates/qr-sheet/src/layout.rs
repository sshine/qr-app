//! Where the stickers sit on the page.
//!
//! Die-cut stock has no cut guides to aim at, so a layout is an origin and a pitch in
//! millimetres rather than a grid with gutters. Getting that wrong wastes a sheet of
//! labels, which is why the presets carry the stock's real geometry and a printer's own
//! offset is corrected by overriding the margins instead of editing one.

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
}

/// DejaVu Sans Mono's advance width, in ems. The caption is set in it, so this is what
/// turns a character count into a width without rendering anything.
const MONO_ADVANCE_EM: f32 = 0.6022;

/// A point in millimetres.
const PT_MM: f32 = 25.4 / 72.0;

/// Below this the caption stops being readable at arm's length, so a layout that would
/// need smaller type is refused rather than printed.
const MIN_CAPTION_PT: f32 = 5.5;

/// Above this the caption competes with the symbol for attention and, on the full-sheet
/// label, would be absurd.
const MAX_CAPTION_PT: f32 = 16.0;

/// The gutter between the symbol and the caption.
pub const CAPTION_GUTTER_MM: f32 = 1.5;

/// The turned address is set at a fixed size rather than one fitted to the label.
///
/// Rotated, it is bounded by the label height rather than by any width, and every stock
/// here is tall enough for it. Fixing it also keeps it out of the circle the caption
/// sizing would otherwise be in: the column it costs decides the width the caption is
/// fitted to, which would otherwise decide its own size.
const HOST_PT: f32 = 5.5;

/// A rotated line is as wide as its type is tall: ascender, descender, and a little
/// either side.
const HOST_EM: f32 = 1.5;

/// Vertical space given to each ruled writing line.
pub const RULE_GAP_MM: f32 = 3.0;

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

    /// What the turned address column costs.
    ///
    /// Always reserved, unlike the secret: the address is where to type, and a sticker
    /// on stock too small for the secret still wants to say where its code lives.
    pub fn host_w_mm(&self) -> f32 {
        HOST_PT * HOST_EM * PT_MM
    }

    /// The size the turned address is set at.
    pub fn host_pt(&self) -> f32 {
        HOST_PT
    }

    /// What is left of the label once the inset, the symbol and the address have taken
    /// their share.
    pub fn caption_w_mm(&self, modules: usize) -> f32 {
        let host = self.host_w_mm() + CAPTION_GUTTER_MM;
        self.label_w_mm - 2.0 * self.inset_x_mm - self.qr_box_mm(modules) - CAPTION_GUTTER_MM - host
    }

    pub fn caption_h_mm(&self) -> f32 {
        self.label_h_mm - 2.0 * self.inset_y_mm
    }

    /// Caption type size, fitted to the column rather than guessed from the label.
    ///
    /// Scaling off the label height instead looks right on the stock it was tuned for
    /// and overflows the one beside it, because a tall label is not necessarily a wide
    /// one. Fitting the longest line to the column that is actually left over cannot
    /// overflow, so the only thing left to check is the lower clamp.
    pub fn caption_pt(&self, modules: usize, longest_line: usize) -> f32 {
        if longest_line == 0 {
            return MIN_CAPTION_PT;
        }
        let unit = mono_width_mm(longest_line, 1.0);
        (self.caption_w_mm(modules) / unit).clamp(MIN_CAPTION_PT, MAX_CAPTION_PT)
    }

    /// The secret is set smaller than the id: the id is what a human reads off the box,
    /// the secret is what they copy only when the camera has failed them.
    pub fn secret_pt(&self, modules: usize, longest_line: usize) -> f32 {
        self.caption_pt(modules, longest_line) * 0.8
    }

    /// How tall the caption comes out, for the fit check.
    fn caption_stack_mm(&self, modules: usize, longest_line: usize, secret_lines: usize) -> f32 {
        let id = self.caption_pt(modules, longest_line) * PT_MM;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a secret is at most a handful of lines"
        )]
        let secret_lines = secret_lines as f32;
        let secret = self.secret_pt(modules, longest_line) * 1.25 * PT_MM * secret_lines;
        let rules = f32::from(self.label_lines) * (RULE_GAP_MM + 0.1);
        id + secret + rules
    }

    /// Refuses a layout that would print off the page, crowd the quiet zone out of a
    /// label, or clip the caption. All three produce a sheet that looks plausible on
    /// screen and wastes a sheet of stock.
    pub fn validate(
        &self,
        name: &str,
        modules: usize,
        longest_line: usize,
        secret_lines: usize,
    ) -> Result<()> {
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

        let caption_w = self.caption_w_mm(modules);
        let wanted = caption_w / mono_width_mm(longest_line.max(1), 1.0);
        if caption_w <= 0.0 || wanted < MIN_CAPTION_PT {
            return Err(Error::CaptionTooWide {
                preset: name.to_owned(),
                caption_w_mm: caption_w,
                needed_pt: MIN_CAPTION_PT,
            });
        }

        if self.caption_stack_mm(modules, longest_line, secret_lines) > self.caption_h_mm() {
            return Err(Error::CaptionTooTall {
                preset: name.to_owned(),
                caption_h_mm: self.caption_h_mm(),
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    /// What a default-length redeem URL encodes to; pinned by crate::qr.
    const MODULES: usize = 33;

    /// The longest line a caption holds: "qr.shine.town", which beats both the grouped
    /// ten-character id and either half of the secret.
    const LONGEST: usize = 13;

    /// The host, then the secret across two lines.
    const SECRET_LINES: usize = 3;

    fn presets() -> Presets {
        Presets::builtin().expect("builtin presets parse")
    }

    #[test]
    fn every_shipped_preset_parses_and_prints_within_its_stock() {
        let presets = presets();
        assert!(presets.names().count() >= 8);
        for (name, layout) in presets.iter() {
            let lines = if layout.print_secret { SECRET_LINES } else { 0 };
            layout
                .validate(name, MODULES, LONGEST, lines)
                .unwrap_or_else(|e| panic!("preset {name} does not fit: {e}"));
        }
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
    fn caption_type_never_falls_below_the_legible_floor() {
        for (name, layout) in presets().iter() {
            let pt = layout.caption_pt(MODULES, LONGEST);
            assert!(
                pt >= MIN_CAPTION_PT,
                "{name} would set the caption at {pt}pt"
            );
            assert!(
                pt <= MAX_CAPTION_PT,
                "{name} would set the caption at {pt}pt"
            );
        }
    }

    #[test]
    fn a_symbol_too_tall_for_its_label_is_refused() {
        let presets = presets();
        let mut layout = presets.get("relief-kopi-48").expect("preset").clone();
        layout.qr_mm = 30.0;
        assert!(layout.validate("overgrown", MODULES, LONGEST, 0).is_err());
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
        };
        let err = layout
            .validate("portrait", MODULES, LONGEST, 0)
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
                .validate("overfull", MODULES, LONGEST, SECRET_LINES)
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
                .validate("overwide", MODULES, LONGEST, SECRET_LINES)
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
            .validate("flush", MODULES, LONGEST, SECRET_LINES)
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
