//! System image decoding and a request-scoped Quick Look view inside the Popup.
use serde::Serialize;
use std::{cell::RefCell, ffi::CString};
use tauri::Emitter;

unsafe extern "C" {
    fn ah_preview_decode_image(
        bytes: *const u8,
        length: usize,
        thumbnail: bool,
        callback: extern "C" fn(*const u8, usize, u32, u32, i32),
    );
    fn ah_preview_update_view(
        window: *mut std::ffi::c_void,
        request: *const std::ffi::c_char,
        path: *const std::ffi::c_char,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        bare_enter: bool,
        callback: extern "C" fn(u16, u64),
    ) -> bool;
}
#[derive(Debug)]
pub enum DecodedImage {
    Png {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
    Multipage {
        count: u32,
    },
}
thread_local! {
    static IMAGE_RESULT: RefCell<Option<Result<DecodedImage, &'static str>>> = const { RefCell::new(None) };
    static KEY_TARGET: RefCell<Option<(tauri::Window, String, usize)>> = const { RefCell::new(None) };
}
extern "C" fn image_result(bytes: *const u8, length: usize, width: u32, height: u32, status: i32) {
    let result = match status {
        0 if !bytes.is_null() && length <= crate::attachment_preview::IMAGE_BYTES => {
            // Swift owns this buffer until the callback returns. Copy only the bounded PNG.
            let bytes = unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec();
            Ok(DecodedImage::Png {
                bytes,
                width,
                height,
            })
        }
        2 => Ok(DecodedImage::Multipage { count: width }),
        3 => Err("limit"),
        4 => Err("unsupported"),
        _ => Err("imageFailed"),
    };
    IMAGE_RESULT.with(|r| *r.borrow_mut() = Some(result));
}
pub fn decode_image(bytes: &[u8], thumbnail: bool) -> Result<DecodedImage, &'static str> {
    unsafe {
        ah_preview_decode_image(bytes.as_ptr(), bytes.len(), thumbnail, image_result);
    }
    IMAGE_RESULT.with(|r| r.borrow_mut().take().unwrap_or(Err("imageFailed")))
}
use crate::attachment_preview::ViewRect;
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PreviewKey {
    request_id: String,
    index: usize,
    key: &'static str,
    meta_key: bool,
}
extern "C" fn native_key(code: u16, flags: u64) {
    let key = match code {
        53 => "Escape",
        13 => "w",
        36 | 76 => "Enter",
        _ => return,
    };
    KEY_TARGET.with(|target| {
        if let Some((window, request_id, index)) = target.borrow().as_ref() {
            if crate::app::popup_preview::request(window, request_id).is_ok() {
                let _ = window.emit(
                    "popup-preview-native-key",
                    PreviewKey {
                        request_id: request_id.clone(),
                        index: *index,
                        key,
                        meta_key: flags & (1 << 20) != 0,
                    },
                );
            }
        }
    });
}
pub fn update_view(
    window: &tauri::Window,
    request_id: &str,
    index: Option<usize>,
    path: Option<&str>,
    rect: Option<ViewRect>,
    bare_enter: bool,
) -> Result<(), String> {
    let pointer = window.ns_window().map_err(|e| e.to_string())?;
    let request = CString::new(request_id).map_err(|_| "invalid request id")?;
    let path = path
        .map(CString::new)
        .transpose()
        .map_err(|_| "invalid attachment path")?;
    let zero = ViewRect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
    let rect = rect.as_ref().unwrap_or(&zero);
    KEY_TARGET.with(|target| {
        *target.borrow_mut() = index.map(|index| (window.clone(), request_id.to_owned(), index))
    });
    let result = unsafe {
        ah_preview_update_view(
            pointer,
            request.as_ptr(),
            path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            bare_enter,
            native_key,
        )
    };
    if result {
        Ok(())
    } else {
        Err("system preview unavailable".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_body_is_clamped_away_from_answer_and_toolbar() {
        let rect = ViewRect {
            x: 0.0,
            y: 0.0,
            width: 2000.0,
            height: 2000.0,
        };
        let body = rect.clipped(560.0, 1266.0, 620.0).unwrap();
        assert_eq!(
            (body.x, body.y, body.width, body.height),
            (566.0, 48.0, 700.0, 572.0)
        );
        assert!(ViewRect {
            x: f64::NAN,
            ..rect.clone()
        }
        .clipped(560.0, 1266.0, 620.0)
        .is_none());
        assert!(ViewRect {
            width: -1.0,
            ..rect
        }
        .clipped(560.0, 1266.0, 620.0)
        .is_none());
    }
    #[test]
    fn native_decoder_rejects_invalid_bytes_and_decodes_real_icons() {
        assert!(decode_image(b"not an image", false).is_err());
        assert!(matches!(
            decode_image(b"icns\x00\x00\x00\x20broken", false),
            Err("imageFailed")
        ));
        for name in ["icon.icns", "icon.ico"] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("icons")
                    .join(name),
            )
            .unwrap();
            match decode_image(&bytes, false).unwrap() {
                DecodedImage::Png {
                    bytes,
                    width,
                    height,
                } => {
                    assert!(bytes.starts_with(b"\x89PNG"));
                    assert!(width >= 256 && height >= 256);
                }
                _ => panic!("icons are representations, not separate pages"),
            }
        }
    }
}
