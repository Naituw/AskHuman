//! Reusable double-action confirmation interaction model and transport.
//!
//! Provides a generic `ConfirmView` that any feature (stage, permission approval, etc.)
//! can produce.  Channel adapters render the view into platform-specific cards and parse
//! callbacks back to a wire-level `ConfirmSlot`; the daemon maps slots to business action
//! IDs via the originating view.

pub mod transport;

use crate::i18n::{self, Lang};

// ─── Generic model ──────────────────────────────────────────────────────────

/// Visual emphasis role for a confirm action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionRole {
    /// Blue / primary emphasis — the positive or default-recommended action.
    Primary,
    /// Red / danger emphasis — a destructive or negative action.
    Destructive,
    /// No special emphasis.
    Default,
}

/// A single action in a two-action confirm card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmAction {
    /// Stable machine-readable identifier (e.g. "approve_once", "stage_confirm").
    pub id: String,
    /// Human-visible button label.
    pub label: String,
    /// Visual role (determines button color/style where the platform supports it).
    pub role: ActionRole,
}

/// Wire-level slot position returned by channel callback parsers.
/// Callers map this back to `ConfirmAction.id` via the originating `ConfirmView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmSlot {
    /// First button (left / top / primary position).
    Primary,
    /// Second button (right / bottom / secondary position).
    Secondary,
}

/// A generic double-action confirmation view (platform-agnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmView {
    pub title: String,
    /// Plain / markdown-ish body (renderers escape as needed).
    pub body: String,
    /// First action (typically the positive/confirm action).
    pub confirm: ConfirmAction,
    /// Second action (typically the cancel/reject action).
    pub cancel: ConfirmAction,
}

impl ConfirmView {
    /// Map a wire slot back to the business action ID.
    pub fn action_id_for_slot(&self, slot: ConfirmSlot) -> &str {
        match slot {
            ConfirmSlot::Primary => &self.confirm.id,
            ConfirmSlot::Secondary => &self.cancel.id,
        }
    }

    /// Convenience: confirm button label.
    pub fn confirm_label(&self) -> &str {
        &self.confirm.label
    }

    /// Convenience: cancel button label.
    pub fn cancel_label(&self) -> &str {
        &self.cancel.label
    }
}

/// Finalized (terminal) state of a confirm card — single disabled label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmFinalView {
    pub title: String,
    pub body: String,
    /// Text shown on the single disabled button (summary of outcome).
    pub label: String,
}

// ─── Business-level confirm request (protocol model) ────────────────────────

/// A complete confirm request submitted via IPC (superset of `ConfirmView`).
/// Contains both the rendering payload and protocol metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    /// Daemon-assigned unique ID for this request.
    pub id: String,
    pub title: String,
    /// Markdown body (may contain details summary).
    pub body_md: String,
    /// Structured details (tool name, command, parameters, etc.) — for richer
    /// rendering where supported; may be truncated per-channel.
    pub details: Option<String>,
    /// First action (positive/confirm).
    pub confirm: ConfirmAction,
    /// Second action (negative/cancel).
    pub cancel: ConfirmAction,
    /// Which action ID does "user dismissed the window" map to (e.g. "deny").
    pub dismiss_action_id: String,
    /// Whether to record this interaction in reply history (permission: false).
    pub record_history: bool,
    /// Absolute expiry (epoch ms); 0 = no expiry.
    pub expires_at_ms: u64,
}

impl ConfirmRequest {
    /// Build a `ConfirmView` suitable for channel rendering.
    pub fn to_view(&self) -> ConfirmView {
        ConfirmView {
            title: self.title.clone(),
            body: self.body_md.clone(),
            confirm: self.confirm.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

/// The structured result of a confirm interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmResult {
    /// The action ID chosen by the user (from `ConfirmAction.id`).
    pub action_id: String,
    /// Which channel delivered the answer ("popup", "feishu", "telegram", etc.).
    pub source_channel_id: String,
}

/// Reason a confirm request ended without a human decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmFallbackReason {
    /// 24-hour timeout reached.
    Expired,
    /// No usable channel (daemon/popup/IM all unavailable).
    NoChannel,
    /// Daemon shutting down / draining.
    DaemonShutdown,
    /// Connection lost before a decision was made.
    ConnectionLost,
}

impl ConfirmFallbackReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::NoChannel => "no_channel",
            Self::DaemonShutdown => "daemon_shutdown",
            Self::ConnectionLost => "connection_lost",
        }
    }
}

// ─── /stage domain builder ──────────────────────────────────────────────────

/// Max paths listed on a stage confirm card.
pub const STAGE_LIST_MAX: usize = 30;

pub const STAGE_CONFIRM_ACTION_ID: &str = "stage_confirm";
pub const STAGE_CANCEL_ACTION_ID: &str = "stage_cancel";

/// Build stage confirm view: list up to `STAGE_LIST_MAX` paths; note remaining count.
pub fn stage_confirm_view(
    lang: Lang,
    project: &str,
    paths: &[String],
    total: usize,
) -> ConfirmView {
    let title = i18n::tr(lang, "confirm.stageTitle").replace("{project}", project);
    let mut body = String::new();
    body.push_str(&i18n::tr(lang, "confirm.stageIntro").replace("{n}", &total.to_string()));
    body.push('\n');
    let show = paths.len().min(STAGE_LIST_MAX).min(total);
    for p in paths.iter().take(show) {
        body.push_str("- ");
        body.push_str(p);
        body.push('\n');
    }
    if total > show {
        body.push_str(
            &i18n::tr(lang, "confirm.stageMore").replace("{n}", &(total - show).to_string()),
        );
        body.push('\n');
    }
    ConfirmView {
        title,
        body,
        confirm: ConfirmAction {
            id: STAGE_CONFIRM_ACTION_ID.to_string(),
            label: i18n::tr(lang, "confirm.btnConfirm").to_string(),
            role: ActionRole::Primary,
        },
        cancel: ConfirmAction {
            id: STAGE_CANCEL_ACTION_ID.to_string(),
            label: i18n::tr(lang, "confirm.btnCancel").to_string(),
            role: ActionRole::Default,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_view_truncates_list() {
        let paths: Vec<String> = (0..40).map(|i| format!("f{i}.rs")).collect();
        let v = stage_confirm_view(Lang::Zh, "proj", &paths, 40);
        assert!(v.body.contains("f0.rs"));
        assert!(v.body.contains("f29.rs"));
        assert!(!v.body.contains("f39.rs"));
        assert!(v.body.contains("10") || v.title.contains("proj"));
    }

    #[test]
    fn action_id_for_slot() {
        let view = ConfirmView {
            title: "t".into(),
            body: "b".into(),
            confirm: ConfirmAction {
                id: "yes".into(),
                label: "Yes".into(),
                role: ActionRole::Primary,
            },
            cancel: ConfirmAction {
                id: "no".into(),
                label: "No".into(),
                role: ActionRole::Destructive,
            },
        };
        assert_eq!(view.action_id_for_slot(ConfirmSlot::Primary), "yes");
        assert_eq!(view.action_id_for_slot(ConfirmSlot::Secondary), "no");
    }

    #[test]
    fn stage_view_action_ids() {
        let v = stage_confirm_view(Lang::En, "proj", &["a.rs".into()], 1);
        assert_eq!(v.confirm.id, STAGE_CONFIRM_ACTION_ID);
        assert_eq!(v.cancel.id, STAGE_CANCEL_ACTION_ID);
        assert_eq!(v.confirm.role, ActionRole::Primary);
        assert_eq!(v.cancel.role, ActionRole::Default);
    }

    #[test]
    fn confirm_request_to_view() {
        let req = ConfirmRequest {
            id: "req-1".into(),
            title: "Approve rm -rf?".into(),
            body_md: "This will delete everything.".into(),
            details: Some("tool: bash".into()),
            confirm: ConfirmAction {
                id: "allow".into(),
                label: "Allow".into(),
                role: ActionRole::Primary,
            },
            cancel: ConfirmAction {
                id: "deny".into(),
                label: "Deny".into(),
                role: ActionRole::Destructive,
            },
            dismiss_action_id: "deny".into(),
            record_history: false,
            expires_at_ms: 0,
        };
        let view = req.to_view();
        assert_eq!(view.title, "Approve rm -rf?");
        assert_eq!(view.confirm.id, "allow");
        assert_eq!(view.cancel.id, "deny");
        assert_eq!(view.confirm.role, ActionRole::Primary);
    }

    #[test]
    fn fallback_reason_as_str() {
        assert_eq!(ConfirmFallbackReason::Expired.as_str(), "expired");
        assert_eq!(ConfirmFallbackReason::NoChannel.as_str(), "no_channel");
        assert_eq!(
            ConfirmFallbackReason::DaemonShutdown.as_str(),
            "daemon_shutdown"
        );
        assert_eq!(
            ConfirmFallbackReason::ConnectionLost.as_str(),
            "connection_lost"
        );
    }
}
