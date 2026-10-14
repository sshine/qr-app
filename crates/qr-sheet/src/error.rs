//! What rendering a sheet fails with.

use std::{io, path::PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("no layout preset named {0}")]
    UnknownPreset(String),

    #[error("the preset file could not be read")]
    PresetSyntax(#[source] toml::de::Error),

    #[error(
        "preset {preset} puts a {qr_mm}mm symbol plus its {quiet_mm}mm quiet zone into a \
         label {label_h_mm}mm tall; shrink qr_mm or pick larger stock"
    )]
    SymbolTooLarge {
        preset: String,
        qr_mm: f32,
        quiet_mm: f32,
        label_h_mm: f32,
    },

    #[error(
        "preset {preset} leaves {caption_w_mm:.1}mm beside the symbol, which cannot hold \
         the printed address at {needed_pt}pt; shrink qr_mm or pick wider stock"
    )]
    CaptionTooWide {
        preset: String,
        caption_w_mm: f32,
        needed_pt: f32,
    },

    #[error(
        "preset {preset} asks for more than the {caption_h_mm:.1}mm box beside the symbol \
         holds; reduce label_lines or turn print_secret off"
    )]
    CaptionTooTall { preset: String, caption_h_mm: f32 },

    #[error(
        "preset {preset} puts {edge} past what a printer will put toner on at the {side} \
         of an A4 page"
    )]
    OffPage {
        preset: String,
        edge: &'static str,
        side: &'static str,
    },

    #[error("the URL for code {id} will not encode as a QR")]
    Encode {
        id: String,
        #[source]
        source: qrcode::types::QrError,
    },

    #[error("typst is not on PATH and no --typst-bin was given")]
    TypstMissing(#[source] io::Error),

    #[error("typst took longer than {0} seconds and was given up on")]
    TypstTimeout(u64),

    #[error("typst refused the document:\n{0}")]
    TypstFailed(String),

    #[error("could not {action} {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(transparent)]
    Core(#[from] qr_core::Error),
}
