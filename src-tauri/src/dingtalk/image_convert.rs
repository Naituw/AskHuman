//! 钉钉图片附件的格式 / 体积适配。
//!
//! 钉钉 `media/upload?type=image` 仅接受 jpg/png/gif/bmp 且单图 ≤ 1MB。本模块在发送前判定：
//! - jpg/png/gif/bmp 且 ≤ 限额 → 原样发送（返回 [`Prepared::Original`]）；
//! - PNG 超限 → 仍编码为 PNG，按需缩放到限额内（保留无损与透明）；
//! - JPEG 超限 → 重新编码 JPEG(q85)，按需缩放到限额内；
//! - 其它不支持格式（webp/…）→ 解码后编码为 JPEG(q85)，按需缩放到限额内；
//! - gif/bmp 超限或任何解码失败 → 回退原样（best-effort，可能仍被钉钉拒绝）。

use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::path::Path;

/// 钉钉单图体积上限（1MB）。留出余量后作为目标，避免临界被拒。
const LIMIT: u64 = 1024 * 1024;
const TARGET: usize = 980 * 1024;
/// JPEG 编码质量。
const JPEG_QUALITY: u8 = 85;

/// 发送前的图片处理结果。
pub enum Prepared {
    /// 原样发送源文件。
    Original,
    /// 发送转码 / 缩放后的字节（`file_name` 决定钉钉识别的扩展名）。
    Converted { bytes: Vec<u8>, file_name: String },
}

fn ext_lower(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string()
}

/// 决定图片如何发送。纯函数（仅读源文件 + 编解码，不做网络）。
pub fn prepare(path: &str, name: &str) -> Prepared {
    let ext = ext_lower(name);
    let supported = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "bmp");
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // 受支持且未超限：原样发送。
    if supported && size <= LIMIT {
        return Prepared::Original;
    }
    // gif/bmp 不重编码（避免破坏动图 / 简化处理）：超限也回退原样（best-effort）。
    if matches!(ext.as_str(), "gif" | "bmp") {
        return Prepared::Original;
    }

    // 需要重编码：png 保持 png；jpeg / 其它不支持格式 → jpeg。
    let img = match image::ImageReader::open(path)
        .ok()
        .and_then(|r| r.with_guessed_format().ok())
        .and_then(|r| r.decode().ok())
    {
        Some(i) => i,
        None => return Prepared::Original, // 解码失败（如 heic）：回退原样。
    };

    let as_png = ext == "png";
    match encode_fit(&img, as_png) {
        Some(bytes) => Prepared::Converted {
            bytes,
            file_name: format!("{}.{}", stem(name), if as_png { "png" } else { "jpg" }),
        },
        None => Prepared::Original,
    }
}

/// 编码到目标体积内：先全尺寸编码，超出则按 0.8 比例逐步缩放重试（最多数轮）。
fn encode_fit(img: &DynamicImage, as_png: bool) -> Option<Vec<u8>> {
    let mut cur = img.clone();
    let mut last: Option<Vec<u8>> = None;
    for _ in 0..6 {
        let buf = if as_png {
            encode_png(&cur)
        } else {
            encode_jpeg(&cur)
        }?;
        if buf.len() <= TARGET {
            return Some(buf);
        }
        let (w, h) = (cur.width(), cur.height());
        let nw = (w as f32 * 0.8) as u32;
        let nh = (h as f32 * 0.8) as u32;
        last = Some(buf);
        if nw < 16 || nh < 16 {
            break;
        }
        cur = cur.resize(nw, nh, image::imageops::FilterType::Lanczos3);
    }
    last
}

fn encode_png(img: &DynamicImage) -> Option<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

fn encode_jpeg(img: &DynamicImage) -> Option<Vec<u8>> {
    // JPEG 不支持 alpha：统一降为 RGB8（透明区合成到黑底，截图通常不透明）。
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut bytes = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
    enc.encode_image(&rgb).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_small_passes_through() {
        // 构造一张极小 PNG 写盘，应原样发送。
        let dir = std::env::temp_dir();
        let p = dir.join("ic_small.png");
        let img = DynamicImage::ImageRgb8(image::RgbImage::new(4, 4));
        img.save(&p).unwrap();
        assert!(matches!(
            prepare(p.to_str().unwrap(), "ic_small.png"),
            Prepared::Original
        ));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn webp_converts_to_jpeg() {
        // 写一张 webp，应被转成 jpg。
        let dir = std::env::temp_dir();
        let p = dir.join("ic_in.webp");
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            32,
            image::Rgb([10, 20, 30]),
        ));
        img.save_with_format(&p, ImageFormat::WebP).unwrap();
        match prepare(p.to_str().unwrap(), "ic_in.webp") {
            Prepared::Converted { file_name, bytes } => {
                assert!(file_name.ends_with(".jpg"));
                assert!(!bytes.is_empty());
            }
            Prepared::Original => panic!("expected conversion for webp"),
        }
        let _ = std::fs::remove_file(p);
    }
}
