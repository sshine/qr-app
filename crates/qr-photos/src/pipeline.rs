//! Turning whatever a phone uploaded into something an album can serve.
//!
//! Three sizes come out of one decode: the original as it arrived, a bounded display
//! copy and a grid thumbnail. Eagerly, during the upload request, because doing it
//! lazily means the first album load after an upload is slower than the upload was and
//! needs a guard against two visitors asking for the same missing size at once.

use std::io::Cursor;

use bytes::Bytes;
use chrono::{DateTime, NaiveDateTime, Utc};
use image::{DynamicImage, ImageFormat, ImageReader, metadata::Orientation};

/// Longest edge of the lightbox copy.
pub const VIEW_EDGE: u32 = 1600;

/// Longest edge of the grid thumbnail.
pub const THUMB_EDGE: u32 = 512;

const VIEW_QUALITY: u8 = 82;
const THUMB_QUALITY: u8 = 80;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PipelineError {
    #[error("that is not an image this service can read")]
    Unrecognised,

    #[error(
        "HEIC photos cannot be read here. On an iPhone, Settings > Camera > Formats > \
         Most Compatible makes the camera produce JPEG instead."
    )]
    Heic,

    #[error("that image is {width}x{height}, which is more than the {limit} pixel limit")]
    TooManyPixels { width: u32, height: u32, limit: u32 },

    #[error("that file is {size} bytes, which is more than the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },

    #[error("that image could not be decoded")]
    Decode(#[source] image::ImageError),

    #[error("a derivative could not be encoded")]
    Encode(#[source] image::ImageError),
}

/// What the magic bytes say, which is the only thing worth believing.
///
/// A multipart part carries whatever content type the client felt like sending, and the
/// one place it matters is deciding what to hand a decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Jpeg,
    Png,
    WebP,
    /// Recognised only so the refusal can say something useful.
    Heic,
}

impl Sniffed {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP => "image/webp",
            Self::Heic => "image/heic",
        }
    }

    fn format(self) -> Option<ImageFormat> {
        match self {
            Self::Jpeg => Some(ImageFormat::Jpeg),
            Self::Png => Some(ImageFormat::Png),
            Self::WebP => Some(ImageFormat::WebP),
            Self::Heic => None,
        }
    }
}

pub fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Sniffed::Jpeg);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Sniffed::Png);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(Sniffed::WebP);
    }
    // ISO base media: a four-byte length, then `ftyp`, then the brand. What an iPhone
    // sends when the camera is left on its default format.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
        ) {
            return Some(Sniffed::Heic);
        }
    }
    None
}

/// What goes into the blob store and the row.
pub struct Processed {
    pub original: Bytes,
    pub view: Bytes,
    pub thumb: Bytes,
    pub media_type: &'static str,
    /// Of the original, after orientation is applied, which is what a viewer sees.
    pub width: u32,
    pub height: u32,
    pub taken_at: Option<DateTime<Utc>>,
}

/// Sizes rather than contents: a failed assertion that printed three JPEGs would be
/// unreadable, and the bytes are not what anyone is debugging.
impl std::fmt::Debug for Processed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Processed")
            .field("media_type", &self.media_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("original_bytes", &self.original.len())
            .field("view_bytes", &self.view.len())
            .field("thumb_bytes", &self.thumb.len())
            .field("taken_at", &self.taken_at)
            .finish()
    }
}

pub struct Limits {
    pub max_bytes: usize,
    pub max_pixels: u32,
    /// Re-encode the original to strip metadata. Off by default, so what is kept is
    /// what the camera produced; on, it also removes where the photo was taken.
    pub strip_original: bool,
}

/// Decodes once and produces every size.
///
/// Blocking. The caller runs it on a thread that is allowed to block, because a twelve
/// megapixel decode and two resizes take a few hundred milliseconds and would otherwise
/// stall every other request sharing the runtime thread.
pub fn process(bytes: Bytes, limits: &Limits) -> Result<Processed, PipelineError> {
    if bytes.len() > limits.max_bytes {
        return Err(PipelineError::TooLarge {
            size: bytes.len(),
            limit: limits.max_bytes,
        });
    }

    let sniffed = sniff(&bytes).ok_or(PipelineError::Unrecognised)?;
    if sniffed == Sniffed::Heic {
        return Err(PipelineError::Heic);
    }
    let format = sniffed.format().ok_or(PipelineError::Unrecognised)?;

    // Dimensions before pixels. A two-megabyte file can declare fifty thousand pixels a
    // side, and decoding it first is how a decompression bomb becomes a four gigabyte
    // allocation.
    let mut reader = ImageReader::new(Cursor::new(bytes.clone()));
    reader.set_format(format);
    let (width, height) = reader.into_dimensions().map_err(PipelineError::Decode)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > u64::from(limits.max_pixels) {
        return Err(PipelineError::TooManyPixels {
            width,
            height,
            limit: limits.max_pixels,
        });
    }

    let orientation = exif_orientation(&bytes);
    let taken_at = exif_taken_at(&bytes);

    let mut reader = ImageReader::new(Cursor::new(bytes.clone()));
    reader.set_format(format);
    let mut decoded = reader.decode().map_err(PipelineError::Decode)?;

    // Applied before resizing, or a portrait photo is thumbnailed to landscape
    // proportions and then rotated into the wrong box.
    if let Some(orientation) = orientation {
        decoded.apply_orientation(orientation);
    }

    let (width, height) = (decoded.width(), decoded.height());

    let view = encode_jpeg(&fit(&decoded, VIEW_EDGE), VIEW_QUALITY)?;
    let thumb = encode_jpeg(&fit(&decoded, THUMB_EDGE), THUMB_QUALITY)?;

    // The derivatives carry no metadata at all, because JPEG encoding writes none. The
    // original is passed through untouched unless asked otherwise, which means /full
    // serves whatever the camera wrote, GPS included.
    let (original, media_type) = if limits.strip_original {
        (encode_jpeg(&decoded, 92)?, "image/jpeg")
    } else {
        (bytes, sniffed.media_type())
    };

    Ok(Processed {
        original,
        view,
        thumb,
        media_type,
        width,
        height,
        taken_at,
    })
}

/// Scales so the longest edge is `edge`, and never upscales: a thumbnail larger than
/// its source is bytes spent on nothing.
fn fit(image: &DynamicImage, edge: u32) -> DynamicImage {
    if image.width() <= edge && image.height() <= edge {
        return image.clone();
    }
    image.resize(edge, edge, image::imageops::FilterType::Lanczos3)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Bytes, PipelineError> {
    let mut out = Vec::new();
    // Alpha has nowhere to go in a JPEG, and leaving it in makes the encoder guess.
    image
        .to_rgb8()
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out, quality,
        ))
        .map_err(PipelineError::Encode)?;
    Ok(Bytes::from(out))
}

fn exif(bytes: &[u8]) -> Option<exif::Exif> {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
}

fn exif_orientation(bytes: &[u8]) -> Option<Orientation> {
    let exif = exif(bytes)?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    let value = field.value.get_uint(0)?;
    Orientation::from_exif(u8::try_from(value).ok()?)
}

/// When the photo was taken, if the camera recorded it.
///
/// Read even when the original keeps its metadata, so the album can order by it and the
/// stripping option does not change what the service knows.
fn exif_taken_at(bytes: &[u8]) -> Option<DateTime<Utc>> {
    let exif = exif(bytes)?;
    let field = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;

    let exif::Value::Ascii(ref values) = field.value else {
        return None;
    };
    let raw = values.first()?;
    let parsed = exif::DateTime::from_ascii(raw).ok()?;

    // EXIF timestamps carry no zone. Treated as UTC rather than guessed at, because the
    // only thing this is used for is ordering and a label.
    NaiveDateTime::parse_from_str(
        &format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            parsed.year, parsed.month, parsed.day, parsed.hour, parsed.minute, parsed.second
        ),
        "%Y-%m-%d %H:%M:%S",
    )
    .ok()
    .map(|naive| naive.and_utc())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn limits() -> Limits {
        Limits {
            max_bytes: 25 * 1024 * 1024,
            max_pixels: 50_000_000,
            strip_original: false,
        }
    }

    /// A real JPEG, encoded here rather than checked in as a fixture.
    fn jpeg(width: u32, height: u32) -> Bytes {
        let mut image = image::RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out, 90,
            ))
            .expect("encode");
        Bytes::from(out)
    }

    #[test]
    fn the_formats_a_camera_and_a_file_picker_produce_are_recognised() {
        assert_eq!(sniff(&jpeg(8, 8)), Some(Sniffed::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some(Sniffed::Png));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some(Sniffed::WebP));
    }

    #[test]
    fn heic_is_recognised_so_the_refusal_can_say_what_to_do() {
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00";
        assert_eq!(sniff(heic), Some(Sniffed::Heic));

        let err = process(Bytes::from_static(heic), &limits()).expect_err("refused");
        assert!(err.to_string().contains("Most Compatible"), "{err}");
    }

    #[test]
    fn anything_that_is_not_an_image_is_refused_on_its_bytes_not_its_claim() {
        for bad in [
            &b"not an image at all"[..],
            &b""[..],
            &b"<svg xmlns='http://www.w3.org/2000/svg'></svg>"[..],
            &b"%PDF-1.7"[..],
        ] {
            assert!(sniff(bad).is_none(), "{bad:?} should not sniff as an image");
        }
    }

    #[test]
    fn a_photo_produces_three_sizes_and_the_derivatives_are_jpeg() {
        let processed = process(jpeg(2400, 1800), &limits()).expect("process");
        assert_eq!(processed.width, 2400);
        assert_eq!(processed.height, 1800);
        assert_eq!(sniff(&processed.view), Some(Sniffed::Jpeg));
        assert_eq!(sniff(&processed.thumb), Some(Sniffed::Jpeg));
        assert!(processed.thumb.len() < processed.view.len());
    }

    #[test]
    fn the_derivatives_are_bounded_and_keep_their_proportions() {
        let processed = process(jpeg(2400, 1800), &limits()).expect("process");
        for (bytes, edge) in [(&processed.view, VIEW_EDGE), (&processed.thumb, THUMB_EDGE)] {
            let decoded = image::load_from_memory(bytes).expect("decode");
            assert!(decoded.width() <= edge && decoded.height() <= edge);
            let ratio = f64::from(decoded.width()) / f64::from(decoded.height());
            assert!((ratio - 2400.0 / 1800.0).abs() < 0.02, "aspect drifted");
        }
    }

    #[test]
    fn a_photo_smaller_than_a_thumbnail_is_not_blown_up() {
        let processed = process(jpeg(200, 150), &limits()).expect("process");
        let thumb = image::load_from_memory(&processed.thumb).expect("decode");
        assert_eq!((thumb.width(), thumb.height()), (200, 150));
    }

    #[test]
    fn the_original_is_kept_byte_for_byte_unless_stripping_was_asked_for() {
        let source = jpeg(64, 64);
        let kept = process(source.clone(), &limits()).expect("process");
        assert_eq!(kept.original, source);
        assert_eq!(kept.media_type, "image/jpeg");

        let stripped = process(
            source.clone(),
            &Limits {
                strip_original: true,
                ..limits()
            },
        )
        .expect("process");
        assert_ne!(stripped.original, source);
    }

    #[test]
    fn a_file_over_the_byte_limit_is_refused_before_anything_is_decoded() {
        let err = process(
            jpeg(64, 64),
            &Limits {
                max_bytes: 16,
                ..limits()
            },
        )
        .expect_err("refused");
        assert!(matches!(err, PipelineError::TooLarge { .. }), "{err}");
    }

    #[test]
    fn a_decompression_bomb_is_refused_on_its_declared_size() {
        // The guard that matters: this file is a few hundred bytes and would decode to
        // far more memory than the limit allows.
        let err = process(
            jpeg(4000, 3000),
            &Limits {
                max_pixels: 1_000_000,
                ..limits()
            },
        )
        .expect_err("refused");
        match err {
            PipelineError::TooManyPixels { width, height, .. } => {
                assert_eq!((width, height), (4000, 3000));
            }
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn every_exif_orientation_maps_to_a_transform() {
        // All eight, because the four that involve a flip are the ones a hand-written
        // table gets wrong, and a sideways photo of a box is the visible symptom.
        for value in 1u8..=8 {
            assert!(
                Orientation::from_exif(value).is_some(),
                "orientation {value} has no transform"
            );
        }
        assert!(Orientation::from_exif(0).is_none());
        assert!(Orientation::from_exif(9).is_none());
    }
}
