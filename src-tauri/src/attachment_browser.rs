//! Fresh, bounded Markdown snapshots opened through the HTTPS browser association.
use std::path::{Path, PathBuf};

pub fn snapshot(path: &str) -> Result<PathBuf, String> {
    if !crate::attachment_markdown::supports(path) {
        return Err("unsupported".into());
    }
    let html = match crate::attachment_preview::load(path) {
        crate::attachment_preview::Content::Markdown { html, .. } => html,
        crate::attachment_preview::Content::Unavailable { reason } => return Err(reason.into()),
        _ => return Err("unsupported".into()),
    };
    let title = Path::new(path)
        .file_name()
        .and_then(|p| p.to_str())
        .unwrap_or("preview");
    let doc = crate::attachment_html::document(title, &html).replacen(
        "<meta charset=\"utf-8\">",
        "<meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src https: http:; base-uri 'none'; form-action 'none'\">",
        1,
    );
    crate::attachment_html::write(path, &doc)
}

// Resolve HTTPS rather than the HTML file association, which may point to an editor.
#[cfg(target_os = "macos")]
pub fn open(path: &Path) -> Result<(), String> {
    crate::macos_menu::open_in_browser(path)
}

#[cfg(target_os = "linux")]
pub fn open(path: &Path) -> Result<(), String> {
    use gio::prelude::*;
    let browser =
        gio::AppInfo::default_for_uri_scheme("https").ok_or("no default browser is configured")?;
    let uri = reqwest::Url::from_file_path(path).map_err(|_| "invalid preview path")?;
    browser
        .launch_uris(&[uri.as_str()], None::<&gio::AppLaunchContext>)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
pub fn open(path: &Path) -> Result<(), String> {
    use std::{os::windows::ffi::OsStringExt, os::windows::process::CommandExt};
    use windows_sys::Win32::UI::Shell::{
        AssocQueryStringW, ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE,
    };
    let protocol: Vec<u16> = "https\0".encode_utf16().collect();
    let mut len = 0;
    unsafe {
        let result = AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_EXECUTABLE,
            protocol.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut len,
        );
        if result < 0 || len == 0 || len > 32768 {
            return Err("no default browser is configured".into());
        }
        let mut executable = vec![0; len as usize];
        if AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_EXECUTABLE,
            protocol.as_ptr(),
            std::ptr::null(),
            executable.as_mut_ptr(),
            &mut len,
        ) != 0
        {
            return Err("unable to resolve the default browser".into());
        }
        let end = executable
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(executable.len());
        let executable = std::ffi::OsString::from_wide(&executable[..end]);
        let uri = reqwest::Url::from_file_path(path).map_err(|_| "invalid preview path")?;
        std::process::Command::new(executable)
            .arg(uri.as_str())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_snapshots_preserve_sources_and_isolate_same_names() {
        let dir = tempfile::tempdir().unwrap();
        // Windows disallows angle brackets in file names; ampersands still exercise title escaping.
        let (filename, escaped_title) = if cfg!(target_os = "windows") {
            ("报告 &one space.MD", "报告 &amp;one space.MD</title>")
        } else {
            ("报告 <one> space.MD", "报告 &lt;one&gt; space.MD</title>")
        };
        let source = dir.path().join(filename);
        let original = "# First\n\n<script>bad()</script>\n\n[x](javascript:alert%281%29) ![secret](file:///secret)";
        std::fs::write(&source, original).unwrap();
        let first = snapshot(source.to_str().unwrap()).unwrap();
        let doc = std::fs::read_to_string(&first).unwrap();
        assert!(doc.contains("<h1>First</h1>"));
        assert!(doc.contains(escaped_title));
        assert!(doc.contains("Content-Security-Policy"));
        assert!(!doc.contains("<script>"));
        assert!(!doc.contains("javascript:"));
        assert!(!doc.contains("file:///secret"));
        assert_eq!(std::fs::read_to_string(&source).unwrap(), original);
        std::fs::write(&source, "# Second").unwrap();
        let second = snapshot(source.to_str().unwrap()).unwrap();
        assert_ne!(first.parent(), second.parent());
        assert_eq!(first.file_name(), second.file_name());
        assert!(std::fs::read_to_string(&second)
            .unwrap()
            .contains("<h1>Second</h1>"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), doc);
        std::fs::remove_dir_all(first.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(second.parent().unwrap()).unwrap();
    }
    #[test]
    fn snapshots_apply_preview_resource_and_type_limits() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("large.md");
        std::fs::write(
            &source,
            vec![b'x'; crate::attachment_preview::TEXT_BYTES + 1],
        )
        .unwrap();
        assert_eq!(snapshot(source.to_str().unwrap()).unwrap_err(), "limit");
        std::fs::write(&source, [0xff, 0x81]).unwrap();
        assert_eq!(snapshot(source.to_str().unwrap()).unwrap_err(), "encoding");
        assert_eq!(
            snapshot(dir.path().join("gone.md").to_str().unwrap()).unwrap_err(),
            "readFailed"
        );
        assert_eq!(
            snapshot(dir.path().join("other.patch").to_str().unwrap()).unwrap_err(),
            "unsupported"
        );
    }
}
