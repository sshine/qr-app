//! Encoding a redeem URL as a symbol, and saying how big that symbol comes out.
//!
//! The whole reason a redeem URL is uppercase lives here: QR picks a mode per segment,
//! and an all-capitals URL encodes in alphanumeric mode at 5.5 bits a character where
//! byte mode would spend 8. The one byte-mode character is the `#` before the secret,
//! which costs a segment header on top of its own eight bits. `budget` is what turns
//! that arithmetic into a number someone can measure against a ruler.

use qr_core::{BaseUrl, CodeId, Secret};
use qrcode::{EcLevel, QrCode, Version, render::svg};

use crate::{Error, Result};

/// Reads an error-correction level from the letter printed in every QR reference.
pub fn parse_ec_level(s: &str) -> Result<EcLevel> {
    match s.trim().to_ascii_uppercase().as_str() {
        "L" => Ok(EcLevel::L),
        "M" => Ok(EcLevel::M),
        "Q" => Ok(EcLevel::Q),
        "H" => Ok(EcLevel::H),
        other => Err(Error::Core(qr_core::Error::Invalid(format!(
            "{other} is not an error-correction level; use L, M, Q or H"
        )))),
    }
}

pub fn ec_level_name(ec: EcLevel) -> &'static str {
    match ec {
        EcLevel::L => "L",
        EcLevel::M => "M",
        EcLevel::Q => "Q",
        EcLevel::H => "H",
    }
}

/// How much damage a level tolerates, for the budget table.
pub fn ec_level_recovery(ec: EcLevel) -> &'static str {
    match ec {
        EcLevel::L => "7%",
        EcLevel::M => "15%",
        EcLevel::Q => "25%",
        EcLevel::H => "30%",
    }
}

/// A rendered symbol and the numbers that decide whether it is printable.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub svg: String,
    /// Modules across, not counting the quiet zone.
    pub modules: usize,
    pub version: i16,
}

/// Encodes `data` and renders it as an SVG one unit per module, with no quiet zone.
///
/// The layout decides how wide the quiet zone is and leaves it as blank paper around the
/// image, so it is not drawn here.
///
/// The unit does not matter: typst scales the document to whatever width the layout asks
/// for, and `shape-rendering="crispEdges"` keeps the module boundaries from being
/// antialiased into grey at print resolution.
pub fn encode(id: &str, data: &str, ec: EcLevel) -> Result<Symbol> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), ec).map_err(|source| {
        Error::Encode {
            id: id.to_owned(),
            source,
        }
    })?;

    let version = match code.version() {
        Version::Normal(v) | Version::Micro(v) => v,
    };

    let svg = code
        .render::<svg::Color<'_>>()
        .quiet_zone(false)
        .module_dimensions(1, 1)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();

    Ok(Symbol {
        svg,
        modules: code.width(),
        version,
    })
}

/// What a given set of lengths costs, measured rather than reasoned about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub id_len: usize,
    pub secret_len: usize,
    pub url_len: usize,
    pub version: i16,
    pub modules: usize,
    pub ec: EcLevel,
}

impl Budget {
    /// Millimetres per module when the symbol is printed `qr_mm` across.
    pub fn mm_per_module(&self, qr_mm: f32) -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a QR is at most 177 modules across, which an f32 holds exactly"
        )]
        let modules = self.modules as f32;
        qr_mm / modules
    }
}

/// Encodes one sample URL at these lengths and reports what came out.
///
/// A sample rather than an exhaustive search: every character of both the id and the
/// secret is drawn from one alphabet that sits entirely inside QR's alphanumeric mode, so
/// the encoded length does not depend on which characters were drawn.
pub fn budget(base: &BaseUrl, id_len: usize, secret_len: usize, ec: EcLevel) -> Result<Budget> {
    let id = CodeId::generate(id_len)?;
    let secret = Secret::generate(secret_len)?;
    let url = base.redeem_url(&id, &secret);
    let symbol = encode(id.as_str(), &url, ec)?;

    Ok(Budget {
        id_len,
        secret_len,
        url_len: url.len(),
        version: symbol.version,
        modules: symbol.modules,
        ec,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use qr_core::ids::{DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN};

    fn base() -> BaseUrl {
        BaseUrl::parse("https://qr.shine.town").expect("base")
    }

    #[test]
    fn the_shipped_lengths_encode_at_version_four_level_q() {
        // Every qr_mm in presets.toml is sized against 33 modules, so moving this makes
        // every mm-per-module figure there wrong.
        let b =
            budget(&base(), DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN, EcLevel::Q).expect("budget");
        assert_eq!(b.url_len, 51);
        assert_eq!(b.version, 4);
        assert_eq!(b.modules, 33);
    }

    #[test]
    fn the_encoded_size_does_not_depend_on_which_characters_were_drawn() {
        let base = base();
        let first =
            budget(&base, DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN, EcLevel::Q).expect("budget");
        for _ in 0..200 {
            let again =
                budget(&base, DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN, EcLevel::Q).expect("budget");
            assert_eq!(first, again);
        }
    }

    #[test]
    fn the_id_length_is_free_across_the_whole_permitted_range() {
        // This is why the default id is ten characters rather than six. A version holds
        // a range of lengths, and at the default secret length every id the schema
        // allows lands inside one version, so the shorter id buys nothing and only
        // makes collisions likelier.
        let base = base();
        for id_len in 6..=12 {
            let b = budget(&base, id_len, DEFAULT_SECRET_LEN, EcLevel::Q).expect("budget");
            assert_eq!(b.modules, 33, "{id_len} characters changed the symbol size");
        }
    }

    #[test]
    fn the_error_correction_level_is_the_only_lever_on_symbol_size() {
        let base = base();
        let sizes: Vec<usize> = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H]
            .into_iter()
            .map(|ec| {
                budget(&base, DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN, ec)
                    .expect("budget")
                    .modules
            })
            .collect();
        assert_eq!(sizes, vec![29, 29, 33, 37]);
    }

    #[test]
    fn a_rendered_symbol_is_svg_and_carries_no_script() {
        let base = base();
        let id = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
        let symbol =
            encode(id.as_str(), &base.redeem_url(&id, &secret), EcLevel::Q).expect("encode");
        assert!(symbol.svg.starts_with("<?xml"));
        assert!(symbol.svg.contains("<svg"));
        assert!(!symbol.svg.contains("<script"));
        assert!(symbol.svg.contains("crispEdges"));
    }

    #[test]
    fn every_level_letter_parses_and_names_itself_again() {
        for letter in ["L", "m", "Q", "h"] {
            let ec = parse_ec_level(letter).expect(letter);
            assert_eq!(
                ec_level_name(ec),
                letter.to_ascii_uppercase(),
                "{letter} did not round-trip"
            );
        }
        assert!(parse_ec_level("Z").is_err());
    }
}
