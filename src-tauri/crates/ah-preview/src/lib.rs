//! Downscaled AVIF previews of image attachments (docs/specs/ios-attachment-traffic.md §6.4).
//!
//! One non-generic entry point, [`make_preview`], takes the original file bytes and returns an AVIF file. Every
//! image-crate type and every generic of the encoder stays inside this crate. That is the whole point of having a
//! crate of its own (AT20): generic code is compiled in the crate that instantiates it, at that crate's optimization
//! level. The repository builds `release` with `opt-level = "z"` and `local-install` with `opt-level = 0`; both are
//! 4–20 times too slow for a decode, a Lanczos resize and an AV1 encode. The root manifest therefore compiles this
//! crate and the codecs it calls with `opt-level = 3` and leaves everything else alone (§10.3 has the measurements).
//!
//! A preview replaces the original on the phone's screen only when it shows the same picture, so every case where it
//! would not is a [`Skip`]: the caller then serves the original. Nothing here is an error to the user.

use std::io::Cursor;

use image::codecs::jpeg::JpegDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, Limits, RgbImage};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
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
    /// Pictures with more pixels than this are not decoded: a few hundred bytes of PNG can describe gigabytes of
    /// pixels, and the decoder, the resize and the encoder each hold a copy.
    pub max_source_pixels: u64,
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

/// Why no preview came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The bytes are not a PNG, JPEG or WebP this build can decode.
    Undecodable,
    /// The picture has more pixels than [`PreviewParams::max_source_pixels`] or exceeds a decoder limit.
    TooLarge,
    /// An animation (APNG, animated WebP). A preview would freeze it.
    Animated,
    /// At least one pixel is not opaque. The AVIF would be flat, and the viewer's background would show through the
    /// original but not through it.
    Transparent,
    /// The picture carries a color profile that cannot be converted to sRGB, the only space the AVIF is tagged with.
    UnsupportedColor,
    /// The AVIF encoder failed.
    EncodeFailed,
    /// The preview is not smaller than the original, so sending it would only add work.
    NotSmaller,
}

/// Memory the decoders may allocate for one picture, on top of [`PreviewParams::max_source_pixels`].
const MAX_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;
/// Neither side of a decoded picture may exceed this.
const MAX_SOURCE_EDGE: u32 = 32_768;

/// Decodes `original`, scales it down to `params.max_edge` and encodes it as AVIF.
pub fn make_preview(original: &[u8], params: &PreviewParams) -> Result<Preview, Skip> {
    match image::guess_format(original) {
        Ok(ImageFormat::Png) => {
            let decoder = PngDecoder::new(Cursor::new(original)).map_err(skip_of)?;
            if decoder.is_apng().map_err(skip_of)? {
                return Err(Skip::Animated);
            }
            preview_from(decoder, original.len(), params)
        }
        Ok(ImageFormat::WebP) => {
            let decoder = WebPDecoder::new(Cursor::new(original)).map_err(skip_of)?;
            if decoder.has_animation() {
                return Err(Skip::Animated);
            }
            preview_from(decoder, original.len(), params)
        }
        Ok(ImageFormat::Jpeg) => {
            let decoder = JpegDecoder::new(Cursor::new(original)).map_err(skip_of)?;
            preview_from(decoder, original.len(), params)
        }
        _ => Err(Skip::Undecodable),
    }
}

fn skip_of(error: ImageError) -> Skip {
    match error {
        ImageError::Limits(_) => Skip::TooLarge,
        _ => Skip::Undecodable,
    }
}

fn preview_from<D: ImageDecoder>(
    mut decoder: D,
    original_len: usize,
    params: &PreviewParams,
) -> Result<Preview, Skip> {
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > params.max_source_pixels {
        return Err(Skip::TooLarge);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    decoder.set_limits(limits).map_err(skip_of)?;

    let has_alpha = decoder.color_type().has_alpha();
    let orientation = decoder.orientation().map_err(skip_of)?;
    let profile = decoder.icc_profile().map_err(|_| Skip::UnsupportedColor)?;
    let mut picture = DynamicImage::from_decoder(decoder).map_err(skip_of)?;
    if has_alpha && has_transparency(&picture) {
        return Err(Skip::Transparent);
    }
    picture.apply_orientation(orientation);

    let scaled = if picture.width().max(picture.height()) > params.max_edge {
        picture.resize(params.max_edge, params.max_edge, FilterType::Lanczos3)
    } else {
        picture
    };
    let mut pixels = scaled.into_rgb8();
    if let Some(profile) = profile {
        to_srgb(&mut pixels, &profile)?;
    }
    encode(&pixels, original_len, params)
}

/// Whether any pixel of `picture` is not fully opaque. A picture with an alpha channel whose pixels are all opaque (a
/// macOS screenshot, for one) has nothing to lose by dropping the channel.
fn has_transparency(picture: &DynamicImage) -> bool {
    match picture {
        DynamicImage::ImageLumaA8(pixels) => pixels.pixels().any(|p| p.0[1] != u8::MAX),
        DynamicImage::ImageRgba8(pixels) => pixels.pixels().any(|p| p.0[3] != u8::MAX),
        DynamicImage::ImageLumaA16(pixels) => pixels.pixels().any(|p| p.0[1] != u16::MAX),
        DynamicImage::ImageRgba16(pixels) => pixels.pixels().any(|p| p.0[3] != u16::MAX),
        DynamicImage::ImageRgba32F(pixels) => pixels.pixels().any(|p| p.0[3] < 1.0),
        _ => false,
    }
}

/// Converts `pixels` from the color space of `icc` to sRGB in place (AT11). Pictures in a space the converter cannot
/// handle are skipped rather than shown with shifted colors.
fn to_srgb(pixels: &mut RgbImage, icc: &[u8]) -> Result<(), Skip> {
    let source = ColorProfile::new_from_slice(icc).map_err(|_| Skip::UnsupportedColor)?;
    if source.color_space != DataColorSpace::Rgb {
        return Err(Skip::UnsupportedColor);
    }
    let transform = source
        .create_transform_8bit(
            Layout::Rgb,
            &ColorProfile::new_srgb(),
            Layout::Rgb,
            TransformOptions::default(),
        )
        .map_err(|_| Skip::UnsupportedColor)?;
    let row_bytes = pixels.width() as usize * 3;
    let mut converted = vec![0u8; pixels.as_raw().len()];
    for (from, to) in pixels
        .as_raw()
        .chunks_exact(row_bytes)
        .zip(converted.chunks_exact_mut(row_bytes))
    {
        transform
            .transform(from, to)
            .map_err(|_| Skip::UnsupportedColor)?;
    }
    let (width, height) = pixels.dimensions();
    *pixels = RgbImage::from_raw(width, height, converted).ok_or(Skip::UnsupportedColor)?;
    Ok(())
}

fn encode(pixels: &RgbImage, original_len: usize, params: &PreviewParams) -> Result<Preview, Skip> {
    let (width, height) = pixels.dimensions();
    let image = imgref::Img::new(pixels.as_raw().as_rgb(), width as usize, height as usize);
    let encoded = ravif::Encoder::new()
        .with_quality(params.quality)
        .with_speed(params.speed)
        .encode_rgb(image)
        .map_err(|_| Skip::EncodeFailed)?;
    if encoded.avif_file.len() >= original_len {
        return Err(Skip::NotSmaller);
    }
    Ok(Preview {
        avif: encoded.avif_file,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder};

    /// Small enough for an AV1 encode in an unoptimized test build, and the fastest speed.
    const PARAMS: PreviewParams = PreviewParams {
        max_edge: 64,
        quality: 76.0,
        speed: 10,
        max_source_pixels: 4_000_000,
    };

    /// A picture that looks a little like a photograph: smooth shapes with noise on top. Its PNG and JPEG are much
    /// bigger than its AVIF, so the "not smaller" rule does not interfere with the tests that are about something else.
    fn picture(width: u32, height: u32) -> RgbImage {
        let mut state = 0x1234_5678_u32;
        RgbImage::from_fn(width, height, |x, y| {
            let mut channel = |phase: f32| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((state >> 24) as f32 / 255.0 - 0.5) * 24.0;
                let wave = ((x as f32 / 9.0 + phase).sin() * (y as f32 / 7.0).cos()) * 90.0;
                (128.0 + wave + noise).clamp(0.0, 255.0) as u8
            };
            image::Rgb([channel(0.0), channel(1.0), channel(2.0)])
        })
    }

    fn png_of(pixels: &RgbImage, profile: Option<Vec<u8>>) -> Vec<u8> {
        let mut info = png::Info::with_size(pixels.width(), pixels.height());
        info.color_type = png::ColorType::Rgb;
        info.bit_depth = png::BitDepth::Eight;
        info.icc_profile = profile.map(std::borrow::Cow::Owned);
        let mut out = Vec::new();
        let mut writer = png::Encoder::with_info(&mut out, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer.write_image_data(pixels.as_raw()).unwrap();
        writer.finish().unwrap();
        out
    }

    fn rgba_png_of(pixels: &RgbImage, alpha: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        let mut raw = Vec::new();
        for (x, y, p) in pixels.enumerate_pixels() {
            raw.extend_from_slice(&[p.0[0], p.0[1], p.0[2], alpha(x, y)]);
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, pixels.width(), pixels.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&raw).unwrap();
        writer.finish().unwrap();
        out
    }

    fn jpeg_of(pixels: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
            .write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    /// `jpeg` with an EXIF block, in front of everything else, that says how to turn the picture upright.
    fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn webp_of(pixels: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    /// An animated WebP: the extended header with the animation flag, the animation chunk and one frame that holds the
    /// lossless still. Enough to be recognized as an animation; nothing decodes its frames here.
    fn animated_webp(still: &[u8], width: u32, height: u32) -> Vec<u8> {
        let frame_chunk = &still[12..];
        let le24 = |v: u32| v.to_le_bytes()[..3].to_vec();
        let chunk = |name: &[u8; 4], body: &[u8]| {
            let mut c = name.to_vec();
            c.extend_from_slice(&(body.len() as u32).to_le_bytes());
            c.extend_from_slice(body);
            if body.len() % 2 == 1 {
                c.push(0);
            }
            c
        };
        let mut vp8x = vec![0b0000_0010, 0, 0, 0];
        vp8x.extend(le24(width - 1));
        vp8x.extend(le24(height - 1));
        let mut anmf = [
            le24(0),
            le24(0),
            le24(width - 1),
            le24(height - 1),
            le24(100),
        ]
        .concat();
        anmf.push(0);
        anmf.extend_from_slice(frame_chunk);
        let mut body = b"WEBP".to_vec();
        body.extend(chunk(b"VP8X", &vp8x));
        body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
        body.extend(chunk(b"ANMF", &anmf));
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    /// The pixel size an AVIF file declares (its `ispe` box), after checking that it is an AVIF file at all.
    fn avif_size(avif: &[u8]) -> (u32, u32) {
        assert_eq!(&avif[4..8], b"ftyp");
        assert_eq!(&avif[8..12], b"avif");
        let at = avif
            .windows(4)
            .position(|w| w == b"ispe")
            .expect("an ispe box");
        let read = |from: usize| u32::from_be_bytes(avif[from..from + 4].try_into().unwrap());
        (read(at + 8), read(at + 12))
    }

    #[test]
    fn a_png_comes_out_as_an_avif_with_its_long_edge_at_the_limit() {
        let original = png_of(&picture(200, 100), None);
        let preview = make_preview(&original, &PARAMS).unwrap();
        assert_eq!((preview.width, preview.height), (64, 32));
        assert_eq!(avif_size(&preview.avif), (64, 32));
        assert!(preview.avif.len() < original.len());
    }

    #[test]
    fn a_jpeg_and_a_webp_are_previewed_too() {
        let pixels = picture(150, 90);
        for original in [jpeg_of(&pixels), webp_of(&pixels)] {
            let preview = make_preview(&original, &PARAMS).unwrap();
            assert_eq!(avif_size(&preview.avif), (64, 38));
        }
    }

    #[test]
    fn a_picture_within_the_limit_is_not_enlarged() {
        let original = png_of(&picture(100, 50), None);
        let preview = make_preview(
            &original,
            &PreviewParams {
                max_edge: 400,
                ..PARAMS
            },
        )
        .unwrap();
        assert_eq!(avif_size(&preview.avif), (100, 50));
    }

    #[test]
    fn the_exif_orientation_is_applied_before_the_scaling() {
        let upright_needs_a_quarter_turn = with_exif_orientation(&jpeg_of(&picture(120, 60)), 6);
        let preview = make_preview(&upright_needs_a_quarter_turn, &PARAMS).unwrap();
        // 120 x 60 turned a quarter is 60 x 120, which scales to 32 x 64.
        assert_eq!(avif_size(&preview.avif), (32, 64));
    }

    #[test]
    fn a_picture_whose_pixels_are_all_opaque_is_previewed_although_it_has_an_alpha_channel() {
        let original = rgba_png_of(&picture(120, 80), |_, _| 255);
        assert!(make_preview(&original, &PARAMS).is_ok());
    }

    #[test]
    fn one_transparent_pixel_is_enough_to_skip_the_preview() {
        let original = rgba_png_of(&picture(120, 80), |x, y| {
            if (x, y) == (60, 40) {
                254
            } else {
                255
            }
        });
        assert_eq!(
            make_preview(&original, &PARAMS).unwrap_err(),
            Skip::Transparent
        );
    }

    #[test]
    fn an_animated_png_is_skipped() {
        let (width, height) = (60, 40);
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(2, 0).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(picture(width, height).as_raw())
            .unwrap();
        writer
            .write_image_data(picture(width, height).as_raw())
            .unwrap();
        writer.finish().unwrap();
        assert_eq!(make_preview(&out, &PARAMS).unwrap_err(), Skip::Animated);
    }

    #[test]
    fn an_animated_webp_is_skipped() {
        let still = webp_of(&picture(40, 30));
        let animated = animated_webp(&still, 40, 30);
        assert_eq!(
            make_preview(&animated, &PARAMS).unwrap_err(),
            Skip::Animated
        );
    }

    #[test]
    fn bytes_that_are_not_a_picture_this_build_decodes_are_undecodable() {
        let png = png_of(&picture(120, 80), None);
        let cases: [&[u8]; 5] = [
            b"",
            b"just some text, certainly not a picture",
            b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff;",
            &png[..png.len() / 2],
            &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0],
        ];
        for bytes in cases {
            assert_eq!(make_preview(bytes, &PARAMS).unwrap_err(), Skip::Undecodable);
        }
    }

    #[test]
    fn a_picture_with_more_pixels_than_the_ceiling_is_too_large_before_it_is_decoded() {
        let original = png_of(&picture(200, 100), None);
        let params = PreviewParams {
            max_source_pixels: 19_999,
            ..PARAMS
        };
        assert_eq!(
            make_preview(&original, &params).unwrap_err(),
            Skip::TooLarge
        );
        let params = PreviewParams {
            max_source_pixels: 20_000,
            ..PARAMS
        };
        assert!(make_preview(&original, &params).is_ok());
    }

    #[test]
    fn a_preview_that_is_not_smaller_than_the_original_is_skipped() {
        let flat = RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 30]));
        let original = png_of(&flat, None);
        assert_eq!(
            make_preview(&original, &PARAMS).unwrap_err(),
            Skip::NotSmaller
        );
    }

    #[test]
    fn a_color_profile_of_another_kind_than_rgb_is_skipped() {
        let gray = ColorProfile::new_gray_with_gamma(2.2).encode().unwrap();
        let original = png_of(&picture(120, 80), Some(gray));
        assert_eq!(
            make_preview(&original, &PARAMS).unwrap_err(),
            Skip::UnsupportedColor
        );
    }

    #[test]
    fn a_picture_with_an_srgb_or_a_p3_profile_is_previewed() {
        for profile in [ColorProfile::new_srgb(), ColorProfile::new_display_p3()] {
            let original = png_of(&picture(120, 80), Some(profile.encode().unwrap()));
            assert!(make_preview(&original, &PARAMS).is_ok());
        }
    }

    #[test]
    fn display_p3_pixels_are_converted_to_srgb_and_srgb_pixels_stay() {
        let from_p3 = ColorProfile::new_display_p3().encode().unwrap();
        let mut pixels = RgbImage::from_pixel(4, 2, image::Rgb([200, 100, 100]));
        to_srgb(&mut pixels, &from_p3).unwrap();
        let converted = pixels.get_pixel(1, 1).0;
        // The same numbers mean a more saturated color in P3 than in sRGB, so the sRGB triple is more extreme.
        assert!(converted[0] >= 205, "{converted:?}");
        assert!(converted[1] <= 97 && converted[2] <= 97, "{converted:?}");

        let from_srgb = ColorProfile::new_srgb().encode().unwrap();
        let mut same = RgbImage::from_pixel(4, 2, image::Rgb([200, 100, 100]));
        to_srgb(&mut same, &from_srgb).unwrap();
        for (a, b) in same.get_pixel(2, 0).0.iter().zip([200u8, 100, 100]) {
            assert!(a.abs_diff(b) <= 1, "{:?}", same.get_pixel(2, 0));
        }
    }

    #[test]
    fn very_long_and_thin_pictures_do_not_break_the_encoder() {
        for (width, height) in [(1, 300), (300, 1), (2, 700), (700, 3), (1, 1)] {
            let original = png_of(&picture(width, height), None);
            // Either a preview comes out or a skip says why; the point is that nothing panics.
            let _ = make_preview(&original, &PARAMS);
        }
    }
}
