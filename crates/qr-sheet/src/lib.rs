//! A4 sticker sheets of QR codes.
//!
//! The codes come in already generated; nothing here touches a database. That is what
//! lets the sheet path be exercised from a test, from the CLI and over HTTP
//! without three copies of the layout arithmetic.

pub mod error;
pub mod layout;
pub mod qr;
pub mod render;

pub use error::{Error, Result};
pub use layout::{Layout, Presets};
pub use qr::{Budget, Symbol, budget, ec_level_name, ec_level_recovery, encode, parse_ec_level};
pub use qrcode::EcLevel;
pub use render::{SheetRenderer, SheetRequest};
