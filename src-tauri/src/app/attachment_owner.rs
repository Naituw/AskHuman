//! Who may preview which attachments.
//!
//! The preview commands never take a file path from the frontend. They take an *owner id* and an
//! attachment index, and the path comes from here:
//!
//! - the Popup window owns the attachments of its active request (owner id = request id);
//! - the History window owns the attachments of one recorded entry (owner id = `history:<entry id>`).
//!
//! The window label and the owner kind must agree: a Popup cannot name a history entry, and the
//! History window cannot name a request. Any other window owns nothing.
use crate::attachment_preview::{
    NativeGeneration, NativePermits, ReadGeneration, HISTORY_OWNER_PREFIX,
};
use crate::models::FileAttachment;
use tauri::{AppHandle, Manager, Window};

pub const POPUP: &str = "popup";
pub const HISTORY: &str = "history";

/// The window an owner id belongs to.
pub fn window_label(owner_id: &str) -> &'static str {
    if owner_id.starts_with(HISTORY_OWNER_PREFIX) {
        HISTORY
    } else {
        POPUP
    }
}

fn history_entry(owner_id: &str) -> Result<&str, String> {
    owner_id
        .strip_prefix(HISTORY_OWNER_PREFIX)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "attachment owner does not belong to this window".to_string())
}

fn unavailable() -> String {
    "attachment preview is unavailable in this window".to_string()
}

/// The owner's attachments. `require_presented` only matters for a Popup, whose request must
/// already be on screen; a history entry has no such phase.
pub fn files(
    window: &Window,
    owner_id: &str,
    require_presented: bool,
) -> Result<Vec<FileAttachment>, String> {
    match window.label() {
        POPUP => super::popup_preview::assigned_request(window, owner_id, require_presented)
            .map(|request| request.message.files),
        HISTORY => crate::history::attachment_files(history_entry(owner_id)?)
            .ok_or_else(|| "history entry no longer exists".to_string()),
        _ => Err(unavailable()),
    }
}

/// One attachment of the owner; the index is checked against the owner's own list.
pub fn file(
    window: &Window,
    owner_id: &str,
    index: usize,
    require_presented: bool,
) -> Result<FileAttachment, String> {
    files(window, owner_id, require_presented)?
        .into_iter()
        .nth(index)
        .ok_or_else(|| "invalid attachment index".to_string())
}

/// Geometry and cancellation only need the owner to belong to the window, never its files.
/// A history entry that was just deleted can therefore still collapse its panel, but cannot
/// read anything: every read goes through [`file`].
pub fn check_window(window: &Window, owner_id: &str) -> Result<(), String> {
    match window.label() {
        POPUP => super::popup_preview::request(window, owner_id).map(|_| ()),
        HISTORY => history_entry(owner_id).map(|_| ()),
        _ => Err(unavailable()),
    }
}

/// Everything the History window leaves behind when it closes: its panel geometry, the read
/// generations and the native-preview permits. A window opened later starts from nothing.
pub fn forget_history(app: &AppHandle) {
    super::popup_preview::reset_history(app);
    if let Some(state) = app.try_state::<ReadGeneration>() {
        state.forget_history();
    }
    if let Some(state) = app.try_state::<NativeGeneration>() {
        state.0.forget_history();
    }
    if let Some(state) = app.try_state::<NativePermits>() {
        state.forget_history();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_ids_name_their_window() {
        assert_eq!(window_label("history:1700000000000-ab"), HISTORY);
        assert_eq!(window_label("0f1e2d3c"), POPUP);
        // Only the prefix decides; the Popup has no id that looks like a history entry.
        assert_eq!(window_label("xhistory:1"), POPUP);
    }

    #[test]
    fn history_owner_ids_need_a_prefix_and_an_entry() {
        assert_eq!(history_entry("history:abc"), Ok("abc"));
        assert!(history_entry("history:").is_err());
        assert!(history_entry("abc").is_err());
        // Entry ids are opaque, so a colon inside one is kept.
        assert_eq!(history_entry("history:a:b"), Ok("a:b"));
    }
}
