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
        "preset {preset} gives the host only {half_w_mm:.1}mm, which cannot hold it at \
         {needed_pt}pt; shrink the host, use narrower stock or pick wider stock"
    )]
    HostTooWide {
        preset: String,
        half_w_mm: f32,
        needed_pt: f32,
    },

    #[error(
        "preset {preset} leaves a {half_w_mm:.1}mm half-label ({content_h_mm:.1}mm tall) \
         with only {qr_box_mm:.1}mm for the symbol; shrink qr_mm or pick larger stock"
    )]
    SymbolTooSmall {
        preset: String,
        qr_box_mm: f32,
        half_w_mm: f32,
        content_h_mm: f32,
    },

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
