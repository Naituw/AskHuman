//! Downscaled AVIF previews of image attachments (docs/specs/ios-attachment-traffic.md §6.4).
//!
//! One non-generic entry point, [`make_preview`], takes the original file bytes and returns an AVIF file. Every
//! image-crate type and every generic of the encoder stays inside this crate. That is the whole point of having a
//! crate of its own (AT20): generic code is compiled in the crate that instantiates it, at that crate's optimization
//! level. The repository builds `release` with `opt-level = "z"` and `local-install` with `opt-level = 0`; both are
//! 4–20 times too slow for a decode, a Lanczos resize and an AV1 encode. The root manifest therefore compiles this
//! crate and the codecs it calls with `opt-level = 3` and leaves everything else alone (§10.3 has the measurements).

use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader};
use rgb::FromSlice;

/// What the caller asks for.
#[derive(Debug, Clone, Copy)]
pub struct PreviewParams {
    /// Longest edge of the preview in pixels. A smaller picture is not enlarged.
    pub max_edge: u32,
    /// AVIF quality, 1.0..=100.0.
    pub quality: f32,
    /// rav1e speed, 1..=10 (higher is faster and larger).
    pub speed: u8,
}

/// A finished preview.
#[derive(Debug, Clone)]
pub struct Preview {
    /// The AVIF file.
    pub avif: Vec<u8>,
    /// Pixel size of the preview.
    pub width: u32,
    pub height: u32,
}

/// Why no preview came out. The caller serves the original instead; none of these is an error to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// The bytes are not a picture this build can decode.
    Undecodable,
    /// The AVIF encoder failed.
    EncodeFailed,
}

/// Decodes `original`, scales it down to `params.max_edge` and encodes it as AVIF.
pub fn make_preview(original: &[u8], params: &PreviewParams) -> Result<Preview, Skip> {
    let reader = ImageReader::new(Cursor::new(original))
        .with_guessed_format()
        .map_err(|_| Skip::Undecodable)?;
    let mut decoder = reader.into_decoder().map_err(|_| Skip::Undecodable)?;
    let orientation = decoder.orientation().map_err(|_| Skip::Undecodable)?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(|_| Skip::Undecodable)?;
    picture.apply_orientation(orientation);

    let scaled = if picture.width().max(picture.height()) > params.max_edge {
        picture.resize(params.max_edge, params.max_edge, FilterType::Lanczos3)
    } else {
        picture
    };
    let rgb = scaled.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let pixels = imgref::Img::new(rgb.as_raw().as_rgb(), width as usize, height as usize);
    let encoded = ravif::Encoder::new()
        .with_quality(params.quality)
        .with_speed(params.speed)
        .encode_rgb(pixels)
        .map_err(|_| Skip::EncodeFailed)?;
    Ok(Preview {
        avif: encoded.avif_file,
        width,
        height,
    })
}
