//! Renders the end-to-end suite's clean QR images with the same encoder and URL shape
//! the service prints on a sheet.
//!
//! A real PNG rather than a synthetic photo: the Playwright camera test feeds these
//! through jsQR exactly as it would feed a frame from a phone, so the decoder test is
//! about the bytes a sticker actually carries.
//!
//!     qr-png <base-url> <codes.json> <output-dir>

use std::{
    env,
    error::Error,
    fs,
    io::{Error as IoError, ErrorKind},
    path::PathBuf,
};

use qr_core::{BaseUrl, CodeId, Secret};
use qrcode::{EcLevel, QrCode};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let base = args.next().ok_or_else(|| missing("base URL"))?;
    let codes_path = args.next().ok_or_else(|| missing("codes.json"))?;
    let out_dir = PathBuf::from(args.next().ok_or_else(|| missing("output directory"))?);

    let base = BaseUrl::parse(&base)?;
    let codes = fs::read_to_string(&codes_path)?;
    let codes: Vec<serde_json::Value> = serde_json::from_str(&codes)?;
    fs::create_dir_all(&out_dir)?;

    for (index, code) in codes.iter().enumerate() {
        let id = field(code, "id")?.parse::<CodeId>()?;
        let secret = field(code, "secret")?.parse::<Secret>()?;
        let url = base.redeem_url(&id, &secret);

        let symbol = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::Q)?;
        let image = symbol
            .render::<image::Luma<u8>>()
            .quiet_zone(true)
            .module_dimensions(12, 12)
            .build();
        image.save(out_dir.join(format!("qr-{index:02}.png")))?;
    }

    println!("wrote {} QR images to {}", codes.len(), out_dir.display());
    Ok(())
}

fn field<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| missing(name).into())
}

fn missing(what: &str) -> IoError {
    IoError::new(
        ErrorKind::InvalidInput,
        format!("expected {what} as an argument or in codes.json"),
    )
}
