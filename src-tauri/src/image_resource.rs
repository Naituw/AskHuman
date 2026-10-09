//! Bounded, header-only image inspection shared by attachment and Markdown image loaders.
use std::{io::Read, path::Path};

pub const IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const SVG_BYTES: usize = 2 * 1024 * 1024;
pub const IMAGE_PIXELS: u64 = 40_000_000;
pub const ANIMATION_PIXELS: u64 = 80_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub display_pixels: u64,
}

pub fn read(path: &Path, max: usize) -> Result<Vec<u8>, &'static str> {
    if !std::fs::metadata(path).map_err(|_| "readFailed")?.is_file() {
        return Err("unsupported");
    }
    let file = std::fs::File::open(path).map_err(|_| "readFailed")?;
    let metadata = file.metadata().map_err(|_| "readFailed")?;
    if !metadata.is_file() {
        return Err("unsupported");
    }
    if metadata.len() > max as u64 {
        return Err("limit");
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(max));
    file.take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "readFailed")?;
    if bytes.len() > max {
        return Err("limit");
    }
    Ok(bytes)
}
pub fn checked_pixels(w: u32, h: u32) -> Result<u64, &'static str> {
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 {
        Err("imageFailed")
    } else if pixels > IMAGE_PIXELS {
        Err("limit")
    } else {
        Ok(pixels)
    }
}
pub(crate) fn svg_dimensions(bytes: &[u8]) -> Result<(u32, u32), &'static str> {
    let source = std::str::from_utf8(bytes).map_err(|_| "encoding")?;
    // No DTD/entity expansion. Display the original only through an isolated image element.
    let doc = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 5000,
        },
    )
    .map_err(|_| "imageFailed")?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err("imageFailed");
    }
    if doc.descendants().take(5001).count() > 5000
        || doc
            .descendants()
            .any(|n| n.ancestors().take(66).count() > 65)
    {
        return Err("limit");
    }
    let dimension = |key| {
        root.attribute(key)
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f64>().ok())
    };
    let view: Vec<f64> = root
        .attribute("viewBox")
        .unwrap_or("")
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    let w = dimension("width")
        .or_else(|| (view.len() == 4).then(|| view[2]))
        .unwrap_or(300.0);
    let h = dimension("height")
        .or_else(|| (view.len() == 4).then(|| view[3]))
        .unwrap_or(150.0);
    if !w.is_finite()
        || !h.is_finite()
        || w < 1.0
        || h < 1.0
        || w > u32::MAX as f64
        || h > u32::MAX as f64
    {
        return Err("imageFailed");
    }
    checked_pixels(w.ceil() as u32, h.ceil() as u32)?;
    Ok((w.ceil() as u32, h.ceil() as u32))
}
pub(crate) fn animation_budget(
    bytes: &[u8],
    format: image::ImageFormat,
    canvas: u64,
) -> Result<u64, &'static str> {
    let mut frames = 0_u64;
    if format == image::ImageFormat::Png {
        let mut i = 8;
        while let Some(header) = bytes.get(i..i + 8) {
            let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
            if &header[4..8] == b"acTL" {
                let count = bytes.get(i + 8..i + 12).ok_or("imageFailed")?;
                frames = u64::from(u32::from_be_bytes(count.try_into().unwrap()));
                if frames > 500 || frames.saturating_mul(canvas) > ANIMATION_PIXELS {
                    return Err("limit");
                }
            }
            i = i
                .checked_add(12 + len)
                .filter(|i| *i <= bytes.len())
                .ok_or("imageFailed")?;
        }
    } else if format == image::ImageFormat::Gif {
        if bytes.len() < 13 {
            return Err("imageFailed");
        }
        let mut i = 13
            + if bytes[10] & 0x80 != 0 {
                3 << ((bytes[10] & 7) + 1)
            } else {
                0
            };
        while i < bytes.len() {
            let block = bytes[i];
            i += 1;
            match block {
                0x3b => break,
                0x21 => {
                    i += 1;
                }
                0x2c => {
                    let header = bytes.get(i..i + 9).ok_or("imageFailed")?;
                    i += 9 + if header[8] & 0x80 != 0 {
                        3 << ((header[8] & 7) + 1)
                    } else {
                        0
                    };
                    i += 1;
                    frames += 1;
                }
                _ => return Err("imageFailed"),
            }
            loop {
                let len = usize::from(*bytes.get(i).ok_or("imageFailed")?);
                i += 1;
                if len == 0 {
                    break;
                }
                i = i
                    .checked_add(len)
                    .filter(|i| *i <= bytes.len())
                    .ok_or("imageFailed")?;
            }
            if frames > 500 || frames.saturating_mul(canvas) > ANIMATION_PIXELS {
                return Err("limit");
            }
        }
    } else if format == image::ImageFormat::WebP {
        let mut i = 12;
        while let Some(header) = bytes.get(i..i + 8) {
            let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            if &header[..4] == b"ANMF" {
                frames += 1;
            }
            i = i
                .checked_add(8 + len + (len & 1))
                .filter(|i| *i <= bytes.len())
                .ok_or("imageFailed")?;
            if frames > 500 || frames.saturating_mul(canvas) > ANIMATION_PIXELS {
                return Err("limit");
            }
        }
    }
    Ok(canvas.saturating_mul(frames.max(1)))
}
pub fn inspect(bytes: &[u8], extension: &str) -> Result<ImageInfo, &'static str> {
    if bytes.len() > IMAGE_BYTES || (extension == "svg" && bytes.len() > SVG_BYTES) {
        return Err("limit");
    }
    let mut display_pixels = 0;
    let (mime, width, height) = if extension == "svg" {
        let (w, h) = svg_dimensions(bytes)?;
        ("image/svg+xml", w, h)
    } else {
        let format = image::guess_format(bytes).map_err(|_| "imageFailed")?;
        let mime = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            image::ImageFormat::Gif => "image/gif",
            image::ImageFormat::WebP => "image/webp",
            image::ImageFormat::Bmp => "image/bmp",
            _ => return Err("unsupported"),
        };
        let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|_| "imageFailed")?;
        let pixels = checked_pixels(w, h)?;
        display_pixels = animation_budget(bytes, format, pixels)?;
        (mime, w, h)
    };
    if display_pixels == 0 {
        display_pixels = u64::from(width) * u64::from(height);
    }
    Ok(ImageInfo {
        mime,
        width,
        height,
        display_pixels,
    })
}
