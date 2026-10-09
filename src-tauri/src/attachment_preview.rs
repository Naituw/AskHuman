//! Bounded readers for attachments already assigned to the current Popup request.
pub use crate::image_resource::IMAGE_BYTES;
use crate::image_resource::{checked_pixels, read};
use base64::Engine;
use serde::Serialize;
use std::path::Path;
#[derive(Default)]
pub struct ReadGeneration(std::sync::Mutex<(String, u64)>);
impl ReadGeneration {
    pub fn invalidate(&self, request: &str, generation: u64) {
        let mut state = self.0.lock().unwrap();
        if state.0 != request {
            *state = (request.to_owned(), generation);
        } else {
            state.1 = state.1.max(generation);
        }
    }
    pub fn current(&self, request: &str, generation: u64) -> bool {
        let state = self.0.lock().unwrap();
        state.0 == request && state.1 == generation
    }
}
pub const TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const NATIVE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_LINES: usize = 20_000;
#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Content {
    Markdown {
        text: String,
        html: String,
    },
    Diff {
        text: String,
        parsed: crate::attachment_diff::ParsedDiff,
    },
    Text {
        text: String,
    },
    Image {
        url: String,
        width: u32,
        height: u32,
    },
    Native {
        image_count: Option<u32>,
    },
    Unavailable {
        reason: &'static str,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub request_id: String,
    pub index: usize,
    pub generation: u64,
    pub content: Content,
}
fn unavailable(reason: &'static str) -> Content {
    Content::Unavailable { reason }
}
fn text(bytes: &[u8], utf16: bool) -> Result<String, &'static str> {
    let text = if utf16 && (bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff])) {
        let little = bytes[0] == 0xff;
        let bytes = &bytes[2..];
        if bytes.len() % 2 != 0 {
            return Err("encoding");
        }
        let units: Vec<_> = bytes
            .chunks_exact(2)
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "encoding")?
    } else {
        if bytes
            .iter()
            .any(|b| *b < 0x20 && !matches!(*b, b'\n' | b'\r' | b'\t'))
        {
            return Err("unsupported");
        }
        std::str::from_utf8(bytes)
            .map_err(|_| "encoding")?
            .strip_prefix('\u{feff}')
            .unwrap_or(std::str::from_utf8(bytes).unwrap())
            .to_owned()
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("unsupported");
    }
    if text.lines().take(MAX_LINES + 1).count() > MAX_LINES {
        return Err("limit");
    }
    Ok(text)
}
fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn image_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "svg"
            | "ico"
            | "tga"
            | "pnm"
            | "ppm"
            | "pgm"
            | "pbm"
            | "pam"
    ) || (cfg!(target_os = "macos") && system_image_extension(ext))
}
fn system_image_extension(ext: &str) -> bool {
    matches!(
        ext,
        "icns"
            | "cur"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "jp2"
            | "j2k"
            | "jpf"
            | "jpx"
            | "psd"
            | "exr"
            | "jxl"
            | "hdr"
            | "sgi"
            | "dds"
            | "ktx"
            | "pict"
            | "pct"
            | "raw"
            | "dng"
            | "cr2"
            | "cr3"
            | "crw"
            | "nef"
            | "nrw"
            | "arw"
            | "srf"
            | "sr2"
            | "orf"
            | "rw2"
            | "raf"
            | "pef"
            | "srw"
            | "3fr"
            | "fff"
            | "iiq"
            | "mos"
            | "kdc"
            | "dcr"
            | "mrw"
    )
}
fn system_document_extension(ext: &str) -> bool {
    matches!(
        ext,
        "pdf"
            | "rtf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "pages"
            | "numbers"
            | "key"
            | "odt"
            | "ods"
            | "odp"
            | "mp3"
            | "m4a"
            | "aac"
            | "wav"
            | "aiff"
            | "aif"
            | "flac"
            | "ogg"
            | "mp4"
            | "m4v"
            | "mov"
            | "avi"
            | "mpeg"
            | "mpg"
            | "webm"
            | "mkv"
            | "usdz"
            | "reality"
            | "epub"
    )
}
fn native_content(path: &Path) -> Result<Content, &'static str> {
    let metadata = std::fs::metadata(path).map_err(|_| "readFailed")?;
    if !metadata.is_file() {
        return Err("unsupported");
    }
    if metadata.len() > NATIVE_BYTES {
        return Err("limit");
    }
    Ok(Content::Native { image_count: None })
}
fn image_content(bytes: Vec<u8>, extension: &str) -> Result<Content, &'static str> {
    #[cfg(target_os = "macos")]
    if system_image_extension(extension) || extension == "ico" {
        return match crate::macos_attachment_preview::decode_image(&bytes, false) {
            Ok(crate::macos_attachment_preview::DecodedImage::Png {
                bytes,
                width,
                height,
            }) => Ok(png_content(bytes, width, height)),
            Ok(crate::macos_attachment_preview::DecodedImage::Multipage { count }) => {
                Ok(Content::Native {
                    image_count: Some(count),
                })
            }
            Err("unsupported") => Ok(Content::Native { image_count: None }),
            Err(reason) => Err(reason),
        };
    }
    if matches!(
        extension,
        "ico" | "tga" | "pnm" | "ppm" | "pgm" | "pbm" | "pam"
    ) {
        let format = if extension == "tga" {
            image::ImageFormat::Tga
        } else {
            image::guess_format(&bytes).map_err(|_| "imageFailed")?
        };
        let reader = || image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
        let (w, h) = reader().into_dimensions().map_err(|_| "imageFailed")?;
        checked_pixels(w, h)?;
        let mut decoder = reader();
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(160 * 1024 * 1024);
        decoder.limits(limits);
        let decoded = decoder.decode().map_err(|_| "imageFailed")?;
        let mut png = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|_| "imageFailed")?;
        if png.get_ref().len() > IMAGE_BYTES {
            return Err("limit");
        }
        return Ok(png_content(png.into_inner(), w, h));
    }
    let crate::image_resource::ImageInfo {
        mime,
        width,
        height,
        ..
    } = crate::image_resource::inspect(&bytes, extension)?;
    Ok(Content::Image {
        url: format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        width,
        height,
    })
}
fn png_content(bytes: Vec<u8>, width: u32, height: u32) -> Content {
    Content::Image {
        url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        width,
        height,
    }
}
pub fn load(path: &str) -> Content {
    let path = Path::new(path);
    let extension = ext(path);
    if cfg!(target_os = "macos") && system_document_extension(&extension) {
        return native_content(path).unwrap_or_else(unavailable);
    }
    let image = image_extension(&extension);
    let max = if image && extension != "svg" {
        IMAGE_BYTES
    } else {
        TEXT_BYTES
    };
    let result = (|| {
        let bytes = read(path, max)?;
        if image {
            return image_content(bytes, &extension);
        }
        let diff = crate::attachment_diff::supports(path.to_str().unwrap_or(""));
        let source = match text(&bytes, !diff) {
            Ok(text) => text,
            Err(reason)
                if cfg!(target_os = "macos")
                    && !diff
                    && !crate::attachment_markdown::supports(path.to_str().unwrap_or(""))
                    && matches!(reason, "unsupported" | "encoding") =>
            {
                return native_content(path)
            }
            Err(reason) => return Err(reason),
        };
        if diff {
            let parsed = crate::attachment_diff::parse(&source).map_err(|_| "limit")?;
            Ok(Content::Diff {
                text: source,
                parsed,
            })
        } else if crate::attachment_markdown::supports(path.to_str().unwrap_or("")) {
            let html = crate::attachment_markdown::render_popup(&source)?;
            if html.len() > 8 * 1024 * 1024 {
                return Err("limit");
            }
            Ok(Content::Markdown { text: source, html })
        } else {
            Ok(Content::Text { text: source })
        }
    })();
    result.unwrap_or_else(unavailable)
}
pub fn thumbnail(path: &str) -> Option<String> {
    if !image_extension(&ext(Path::new(path))) {
        return None;
    }
    let bytes = read(Path::new(path), TEXT_BYTES).ok()?;
    #[cfg(target_os = "macos")]
    if system_image_extension(&ext(Path::new(path))) || ext(Path::new(path)) == "ico" {
        return match crate::macos_attachment_preview::decode_image(&bytes, true).ok()? {
            crate::macos_attachment_preview::DecodedImage::Png {
                bytes,
                width,
                height,
            } => match png_content(bytes, width, height) {
                Content::Image { url, .. } => Some(url),
                _ => None,
            },
            _ => None,
        };
    }
    match image_content(bytes, &ext(Path::new(path))).ok()? {
        Content::Image { url, width, height }
            if u64::from(width) * u64::from(height) <= 4_000_000 =>
        {
            Some(url)
        }
        _ => None,
    }
}

#[derive(Clone, serde::Deserialize)]
pub struct ViewRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl ViewRect {
    pub fn clipped(&self, main_width: f64, width: f64, height: f64) -> Option<Self> {
        if ![
            self.x,
            self.y,
            self.width,
            self.height,
            main_width,
            width,
            height,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            return None;
        }
        // Never let a frontend-provided frame cover the answering area or fixed toolbar.
        let x = self.x.max(main_width + 6.0).min(width);
        let y = self.y.max(48.0).min(height);
        let right = (self.x + self.width).min(width);
        let bottom = (self.y + self.height).min(height);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

#[derive(Default)]
pub struct NativeGeneration(pub ReadGeneration);
type NativeStamp = (u64, Option<std::time::SystemTime>);
type NativePermitMap = (String, std::collections::HashMap<usize, NativeStamp>);
#[derive(Default)]
pub struct NativePermits(std::sync::Mutex<NativePermitMap>);
impl NativePermits {
    pub fn allow(&self, request: &str, index: usize, path: &str) {
        let Ok(metadata) = std::fs::metadata(path) else {
            return;
        };
        let mut state = self.0.lock().unwrap();
        if state.0 != request {
            *state = (request.to_owned(), Default::default());
        }
        state
            .1
            .insert(index, (metadata.len(), metadata.modified().ok()));
    }
    pub fn check(&self, request: &str, index: usize, path: &str) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        let state = self.0.lock().unwrap();
        metadata.is_file()
            && metadata.len() <= NATIVE_BYTES
            && state.0 == request
            && state.1.get(&index) == Some(&(metadata.len(), metadata.modified().ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_resource::{animation_budget, svg_dimensions};
    #[test]
    #[ignore = "manual resource benchmark"]
    fn preview_resource_benchmark() {
        let dir = tempfile::tempdir().unwrap();
        let dense = format!(
            "--- a/x\n+++ b/x\n@@ -1,19000 +1,19000 @@\n{}",
            (0..19000)
                .map(|i| format!(" {} {i}\n", "x".repeat(85)))
                .collect::<String>()
        );
        let table = format!(
            "| a | b |\n|---|---|\n{}",
            "| value | value |\n".repeat(19000)
        );
        let cases = [
            ("dense.patch", dense),
            ("long.json", "x".repeat(TEXT_BYTES)),
            ("tables.md", table),
            (
                "large.md",
                "# Heading\n\n".to_owned() + &"text ".repeat(400000),
            ),
        ];
        for (name, source) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, source.as_bytes()).unwrap();
            let start = std::time::Instant::now();
            let content = load(path.to_str().unwrap());
            let elapsed = start.elapsed();
            let encoded = serde_json::to_vec(&content).unwrap();
            println!(
                "{name}: input={} bytes, result={} bytes, elapsed={:.2}ms, unavailable={}",
                source.len(),
                encoded.len(),
                elapsed.as_secs_f64() * 1000.0,
                matches!(content, Content::Unavailable { .. })
            );
        }
        let start = std::time::Instant::now();
        let image = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../assets/overview.webp"
        ));
        assert!(matches!(image, Content::Image { .. }));
        println!(
            "overview.webp: elapsed={:.2}ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    #[test]
    fn queued_generation_invalidates_and_resets_for_new_requests() {
        let epoch = ReadGeneration::default();
        epoch.invalidate("first", 4);
        epoch.invalidate("first", 3);
        assert!(epoch.current("first", 4));
        assert!(!epoch.current("first", 3));
        epoch.invalidate("second", 1);
        assert!(epoch.current("second", 1));
        assert!(!epoch.current("first", 4));
    }
    #[test]
    fn animations_keep_the_original_bytes_and_enforce_canvas_budget() {
        use image::{Frame, RgbaImage};
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            encoder
                .encode_frame(Frame::new(RgbaImage::new(20, 10)))
                .unwrap();
            encoder
                .encode_frame(Frame::new(RgbaImage::new(20, 10)))
                .unwrap();
        }
        assert!(animation_budget(&bytes, image::ImageFormat::Gif, 200).is_ok());
        assert_eq!(
            animation_budget(&bytes, image::ImageFormat::Gif, 50_000_000),
            Err("limit")
        );
        match image_content(bytes.clone(), "gif").unwrap() {
            Content::Image { url, width, height } => {
                assert_eq!((width, height), (20, 10));
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(url.split_once(',').unwrap().1)
                        .unwrap(),
                    bytes
                );
            }
            _ => panic!("expected image"),
        }
    }
    #[test]
    fn reliable_text_encoding_and_binary_detection() {
        assert_eq!(text(b"\xef\xbb\xbfhello", true).unwrap(), "hello");
        assert_eq!(text(&[0xff, 0xfe, 0x2d, 0x4e], true).unwrap(), "中");
        assert_eq!(text(&[0xfe, 0xff, 0x4e, 0x2d], true).unwrap(), "中");
        assert!(text(&[0xff, 0xfe, 0x00, 0xd8], true).is_err());
        assert_eq!(text(b"a\0b", true).unwrap_err(), "unsupported");
        assert_eq!(text(&[0xff, 0, 1], true).unwrap_err(), "unsupported");
        assert!(text(&[0xff, 0xfe, 1], true).is_err());
        assert!(text(&[0xff, 0xfe, 1, 0], false).is_err());
    }
    #[test]
    fn bounded_reads_and_shared_diff_model() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("review.PATCH");
        std::fs::write(
            &p,
            include_str!("../tests/fixtures/attachment-preview.patch"),
        )
        .unwrap();
        assert!(matches!(load(p.to_str().unwrap()), Content::Diff { .. }));
        std::fs::write(&p, vec![b'x'; TEXT_BYTES + 1]).unwrap();
        assert!(matches!(
            load(p.to_str().unwrap()),
            Content::Unavailable { reason: "limit" }
        ));
        assert!(matches!(
            load(dir.path().to_str().unwrap()),
            Content::Unavailable {
                reason: "unsupported"
            }
        ));
    }
    #[test]
    fn image_header_limits_and_svg_structure() {
        assert!(checked_pixels(10000, 10000).is_err());
        assert_eq!(
            svg_dimensions(b"<svg viewBox='0 0 640 480'/>").unwrap(),
            (640, 480)
        );
        assert!(svg_dimensions(b"<svg width='100000' height='100000'/>").is_err());
        assert!(svg_dimensions(b"<!DOCTYPE svg [<!ENTITY x 'hi'>]><svg>&x;</svg>").is_err());
    }
    #[test]
    fn lightweight_formats_are_png_previews_without_changing_transport_classification() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.ppm");
        std::fs::write(&path, b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00").unwrap();
        match load(path.to_str().unwrap()) {
            Content::Image { url, width, height } => {
                assert_eq!((width, height), (2, 1));
                assert!(url.starts_with("data:image/png;base64,"));
            }
            _ => panic!("PNM must be decoded into a browser-supported format"),
        }
        for name in ["image.ico", "image.icns", "image.psd", "image.tiff"] {
            assert!(!crate::cli::file_attachment::is_image_ext(name));
        }
    }
    #[test]
    fn native_permits_cannot_be_reused_for_a_different_request_or_changed_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.pdf");
        std::fs::write(&path, b"original").unwrap();
        let path = path.to_str().unwrap();
        let permits = NativePermits::default();
        assert!(!permits.check("r", 0, path));
        permits.allow("r", 0, path);
        assert!(permits.check("r", 0, path));
        assert!(!permits.check("other", 0, path));
        assert!(!permits.check("r", 1, path));
        std::fs::write(path, b"changed document").unwrap();
        assert!(!permits.check("r", 0, path));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn system_formats_preserve_native_documents_and_multipage_images() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["document.PDF", "rich.rtf", "office.docx", "audio.wav"] {
            let path = directory.path().join(name);
            std::fs::write(&path, b"test content").unwrap();
            assert!(matches!(
                load(path.to_str().unwrap()),
                Content::Native { .. }
            ));
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(NATIVE_BYTES + 1)
                .unwrap();
            assert!(matches!(
                load(path.to_str().unwrap()),
                Content::Unavailable { reason: "limit" }
            ));
        }
        assert!(matches!(
            load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/preview-multipage.tiff"
            )),
            Content::Native {
                image_count: Some(2)
            }
        ));
        // Missing Image I/O codecs can still delegate to a system preview extension;
        // recognized but corrupt image containers must retain their failure message.
        let path = directory.path().join("provider.pict");
        std::fs::write(&path, b"opaque preview provider payload").unwrap();
        assert!(matches!(
            load(path.to_str().unwrap()),
            Content::Native { .. }
        ));
        let path = directory.path().join("corrupt.icns");
        std::fs::write(&path, b"icns\x00\x00\x00\x20broken").unwrap();
        assert!(matches!(
            load(path.to_str().unwrap()),
            Content::Unavailable {
                reason: "imageFailed"
            }
        ));
        // Rich text is explicitly native, while HTML and code remain inert text.
        let path = directory.path().join("page.html");
        std::fs::write(&path, b"<script>active()</script>").unwrap();
        assert!(matches!(load(path.to_str().unwrap()), Content::Text { .. }));
    }
}
