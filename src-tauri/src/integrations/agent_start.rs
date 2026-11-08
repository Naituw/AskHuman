//! Whether a task can be started on an Agent right now: the one rule behind the phone's "New task"
//! page, IM `/new`, the Settings readiness matrix and `AskHuman doctor`, so that no surface can
//! call an Agent unstartable while another one starts it.
//!
//! An Agent is judged by the way it would be started. Codex and Cursor have two routes (their
//! desktop app and the CLI in a terminal) and whichever their launch status resolves to counts: a
//! Mac that has only the Codex Desktop App, or only Cursor's Agents window, starts tasks all the
//! same. Every other family has the CLI route alone and is judged by `AgentReadiness::ready`. A
//! family that cannot launch at all (Antigravity) is never startable.

use serde::Serialize;

use super::agent_launch::AgentReadiness;
use crate::agents::AgentKind;
use crate::codex_desktop::{self, LaunchTarget};
use crate::cursor_desktop;

/// The route a task on `item` would run on, or `None` when none can be started now.
pub fn route(
    item: &AgentReadiness,
    codex: &codex_desktop::LaunchStatus,
    cursor: &cursor_desktop::LaunchStatus,
) -> Option<LaunchTarget> {
    if !item.launch_supported {
        return None;
    }
    match item.kind {
        AgentKind::Codex => codex.target,
        AgentKind::Cursor => cursor.target,
        _ => item.ready.then_some(LaunchTarget::Terminal),
    }
}

/// Whether a task can be started on `item` right now.
pub fn startable(
    item: &AgentReadiness,
    codex: &codex_desktop::LaunchStatus,
    cursor: &cursor_desktop::LaunchStatus,
) -> bool {
    route(item, codex, cursor).is_some()
}

/// [`startable`] for one Agent, reading the launch status it needs from this machine. For a call
/// that judges a single Agent; a batch should read the statuses once and use [`startable`].
pub fn startable_now(item: &AgentReadiness) -> bool {
    if !item.launch_supported {
        return false;
    }
    match item.kind {
        AgentKind::Codex => codex_desktop::launch_status(false).target.is_some(),
        AgentKind::Cursor => cursor_desktop::launch_status(false).target.is_some(),
        _ => item.ready,
    }
}

/// One Agent's readiness together with the verdict, as Settings and `doctor` read it. The
/// readiness fields stay where they were; the rest is added beside them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStart {
    #[serde(flatten)]
    pub readiness: AgentReadiness,
    /// Whether a task can be started on this Agent now (see [`route`]).
    pub startable: bool,
    /// Where it would run; `None` when it cannot start.
    pub start_route: Option<LaunchTarget>,
    /// Codex and Cursor: the launch status behind the verdict (both routes and why each is
    /// unavailable), in the shape of `codex_launch_status` / `cursor_launch_status`.
    pub launch: Option<serde_json::Value>,
}

/// Judges `items`. The launch statuses are read only when Codex or Cursor is among them, and from
/// the probe cache the items were just read from, so no login shell is started a second time.
pub fn judge(items: Vec<AgentReadiness>) -> Vec<AgentStart> {
    let has = |kind: AgentKind| items.iter().any(|item| item.kind == kind);
    let codex = has(AgentKind::Codex).then(|| codex_desktop::launch_status(false));
    let cursor = has(AgentKind::Cursor).then(|| cursor_desktop::launch_status(false));
    items
        .into_iter()
        .map(|item| {
            let (start_route, launch) = match item.kind {
                _ if !item.launch_supported => (None, None),
                AgentKind::Codex => match &codex {
                    Some(status) => (status.target, serde_json::to_value(status).ok()),
                    None => (None, None),
                },
                AgentKind::Cursor => match &cursor {
                    Some(status) => (status.target, serde_json::to_value(status).ok()),
                    None => (None, None),
                },
                _ => (item.ready.then_some(LaunchTarget::Terminal), None),
            };
            AgentStart {
                startable: start_route.is_some(),
                start_route,
                launch,
                readiness: item,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: AgentKind, ready: bool) -> AgentReadiness {
        AgentReadiness {
            kind,
            label: kind.label().to_string(),
            command: String::new(),
            executable: None,
            version: None,
            binary_ready: ready,
            lifecycle_ready: ready,
            integration_ready: ready,
            integration_mode: "cli".into(),
            ready,
            launch_supported: crate::integrations::agent_launch::launch_supported(kind),
            diagnostics: Vec::new(),
            protocol_minimum_version: None,
            protocol_supported: true,
        }
    }

    fn codex_status(target: Option<LaunchTarget>) -> codex_desktop::LaunchStatus {
        codex_desktop::LaunchStatus {
            preference: LaunchTarget::Desktop,
            target,
            desktop_available: target == Some(LaunchTarget::Desktop),
            terminal_available: target == Some(LaunchTarget::Terminal),
            integrated: true,
            desktop_reason: None,
            terminal_reason: None,
        }
    }

    fn cursor_status(target: Option<LaunchTarget>) -> cursor_desktop::LaunchStatus {
        cursor_desktop::LaunchStatus {
            preference: LaunchTarget::Terminal,
            target,
            desktop_available: target == Some(LaunchTarget::Desktop),
            terminal_available: target == Some(LaunchTarget::Terminal),
            integrated: true,
            desktop_reason: None,
            terminal_reason: None,
        }
    }

    #[test]
    fn codex_and_cursor_start_by_their_route_whatever_the_cli_says() {
        let codex = codex_status(Some(LaunchTarget::Desktop));
        let cursor = cursor_status(Some(LaunchTarget::Desktop));
        // The CLI of both is missing, yet each has a route that works.
        assert_eq!(
            route(&item(AgentKind::Codex, false), &codex, &cursor),
            Some(LaunchTarget::Desktop)
        );
        assert_eq!(
            route(&item(AgentKind::Cursor, false), &codex, &cursor),
            Some(LaunchTarget::Desktop)
        );
        // And the CLI being ready does not start an Agent whose routes are both unusable.
        let none_codex = codex_status(None);
        let none_cursor = cursor_status(None);
        assert!(!startable(
            &item(AgentKind::Codex, true),
            &none_codex,
            &none_cursor
        ));
        assert!(!startable(
            &item(AgentKind::Cursor, true),
            &none_codex,
            &none_cursor
        ));
    }

    #[test]
    fn the_other_families_start_by_their_own_readiness() {
        let codex = codex_status(Some(LaunchTarget::Desktop));
        let cursor = cursor_status(Some(LaunchTarget::Desktop));
        for kind in [AgentKind::Claude, AgentKind::Grok, AgentKind::Pi] {
            assert_eq!(
                route(&item(kind, true), &codex, &cursor),
                Some(LaunchTarget::Terminal),
                "{kind:?}"
            );
            assert_eq!(route(&item(kind, false), &codex, &cursor), None, "{kind:?}");
        }
    }

    #[test]
    fn a_family_that_cannot_launch_never_starts() {
        let codex = codex_status(Some(LaunchTarget::Terminal));
        let cursor = cursor_status(Some(LaunchTarget::Terminal));
        assert_eq!(
            route(&item(AgentKind::Antigravity, true), &codex, &cursor),
            None
        );
    }

    #[test]
    fn the_verdict_is_added_beside_the_readiness_fields() {
        let value = serde_json::to_value(AgentStart {
            readiness: item(AgentKind::Claude, true),
            startable: true,
            start_route: Some(LaunchTarget::Terminal),
            launch: None,
        })
        .unwrap();
        // The fields the readiness always had are still at the top level.
        assert_eq!(value["kind"], "claude");
        assert_eq!(value["ready"], true);
        assert_eq!(value["launchSupported"], true);
        assert_eq!(value["startable"], true);
        assert_eq!(value["startRoute"], "terminal");
        assert_eq!(value["launch"], serde_json::Value::Null);
    }
}
