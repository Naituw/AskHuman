//! The popup window's native title on Windows and Linux (docs/specs/popup-session-info.md).
//!
//! macOS hides the native title and draws the Agent session title inside the page, in the strip the
//! traffic lights leave empty. Elsewhere the page has no spare strip, so the session title goes into
//! the native title as `<session title> — AskHuman`; that also tells several popup windows apart in
//! the taskbar and the window switcher.
//!
//! The page only reports the session title. The text of the native title, and putting it back after
//! a settings change, are decided here so there is a single owner and the language reset in
//! `save_settings` cannot wipe the session title.

use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager};

use crate::i18n::{self, Lang};

/// A title bar shows a few dozen characters at most; an unbounded string is of no use there.
const MAX_CHARS: usize = 120;

/// The session title of the popup that is on screen. The shared popup window is reused across
/// requests, so this is a single slot rather than per-request state.
static SESSION_TITLE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<String>> {
    SESSION_TITLE.get_or_init(|| Mutex::new(None))
}

/// Characters that must never reach a native title: control characters and the bidirectional
/// overrides that could reorder the ` — AskHuman` suffix.
fn is_unsafe(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A single-line, trimmed, bounded session title; `None` when nothing meaningful is left.
pub fn normalize(raw: &str) -> Option<String> {
    // Split on whitespace first: tabs and newlines are control characters too, and dropping them
    // before the split would glue neighbouring words together.
    let one_line = raw
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !is_unsafe(*c)).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if one_line.is_empty() {
        return None;
    }
    if one_line.chars().count() <= MAX_CHARS {
        return Some(one_line);
    }
    let mut cut: String = one_line.chars().take(MAX_CHARS).collect();
    cut.push('…');
    Some(cut)
}

/// `<session title> — <app title>`, or just the app title when there is no session title.
pub fn compose(app_title: &str, session: Option<&str>) -> String {
    match session {
        Some(s) if !s.is_empty() => format!("{s} — {app_title}"),
        _ => app_title.to_string(),
    }
}

fn current_title(lang: Lang) -> String {
    let session = slot().lock().ok().and_then(|s| s.clone());
    crate::development::window_title(&compose(i18n::tr(lang, "title.popup"), session.as_deref()))
}

/// Report the Agent session title of the popup that is on screen; `None` or a blank title means it
/// has none. Only the popup window may call this.
#[tauri::command]
pub fn popup_set_session_title(window: tauri::Window, title: Option<String>) -> Result<(), String> {
    if window.label() != "popup" {
        return Err("only the popup window can set its session title".into());
    }
    let next = title.as_deref().and_then(normalize);
    *slot()
        .lock()
        .map_err(|_| "popup title state is unavailable".to_string())? = next;
    // Always apply, even when the stored title did not change: the window may have been recreated
    // with the plain title while this process kept running, and re-setting the same text is free.
    window
        .set_title(&current_title(Lang::current()))
        .map_err(|e| e.to_string())
}

/// Set the popup window's title again while keeping the session title; used after a settings
/// change may have switched the interface language.
pub fn refresh(app: &AppHandle, lang: Lang) {
    if let Some(window) = app.get_webview_window("popup") {
        let _ = window.set_title(&current_title(lang));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_collapses_whitespace_into_one_line() {
        assert_eq!(
            normalize("  Fix the\tpopup \n title  ").as_deref(),
            Some("Fix the popup title")
        );
    }

    #[test]
    fn normalize_drops_blank_titles() {
        assert_eq!(normalize(""), None);
        assert_eq!(normalize(" \n\t "), None);
        assert_eq!(normalize("\u{0}\u{7}"), None);
    }

    #[test]
    fn normalize_strips_control_and_bidi_override_characters() {
        assert_eq!(
            normalize("a\u{0}b\u{202E}c\u{2066}d").as_deref(),
            Some("abcd")
        );
        // A word made only of such characters must not leave a double space behind.
        assert_eq!(normalize("a \u{7} b").as_deref(), Some("a b"));
    }

    #[test]
    fn normalize_keeps_a_title_at_the_limit_unchanged() {
        let exact = "字".repeat(MAX_CHARS);
        assert_eq!(normalize(&exact).as_deref(), Some(exact.as_str()));
    }

    #[test]
    fn normalize_cuts_long_titles_by_characters_not_bytes() {
        let cut = normalize(&"字".repeat(MAX_CHARS + 50)).unwrap();
        assert_eq!(cut.chars().count(), MAX_CHARS + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn compose_puts_the_session_title_before_the_app_title() {
        assert_eq!(
            compose("AskHuman", Some("Fix the popup")),
            "Fix the popup — AskHuman"
        );
    }

    #[test]
    fn compose_without_a_session_title_is_just_the_app_title() {
        assert_eq!(compose("AskHuman", None), "AskHuman");
        assert_eq!(compose("AskHuman", Some("")), "AskHuman");
    }
}
