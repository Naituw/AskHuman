//! The setup wizard's optional checks (`docs/specs/onboarding.md` D11, plan §4).
//!
//! A *probe* is an ordinary request the wizard sends to itself: to the desktop popup, or to
//! exactly one remote channel, so the person can see that the path works. The request carries a
//! marker in its native id. This module names the marker, decides what the daemon does
//! differently with a marked request, builds the question, and maps its answer to an outcome.
//! Running the checks (the ask, the wait, the Agent observation) is in `probe_run`.

use crate::i18n::Lang;
use crate::ipc::{PendingRequestInfo, TaskRequest};
use crate::models::{MessagePrompt, OptionItem, OutputFormat, Question};
use crate::onboarding::channels;
use std::collections::HashSet;

/// Start of the native id every probe carries.
pub const PREFIX: &str = "setup-probe:";

/// Where a probe is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeTarget {
    /// The desktop popup only.
    Popup,
    /// One remote channel only (`telegram` | `slack` | `dingding` | `feishu`).
    Channel(&'static str),
}

impl ProbeTarget {
    fn name(self) -> &'static str {
        match self {
            ProbeTarget::Popup => "popup",
            ProbeTarget::Channel(id) => id,
        }
    }

    /// `popup` or one of the wizard's channel ids.
    pub fn parse(name: &str) -> Option<ProbeTarget> {
        if name == "popup" {
            return Some(ProbeTarget::Popup);
        }
        channels::ORDER
            .iter()
            .copied()
            .find(|id| *id == name)
            .map(ProbeTarget::Channel)
    }
}

/// Which remote channels a request reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImScope {
    /// Whatever the ordinary delivery rules pick.
    All,
    /// None.
    Nothing,
    /// Exactly this channel, whatever the active-channel rules say.
    Only(&'static str),
}

impl ImScope {
    /// The channel a scoped request is limited to.
    pub fn only(self) -> Option<&'static str> {
        match self {
            ImScope::Only(id) => Some(id),
            _ => None,
        }
    }
}

/// What the daemon does with a request; the ordinary request is the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbePolicy {
    /// Show it in the desktop popup.
    pub popup: bool,
    /// Which remote channels get it.
    pub im: ImScope,
    /// Fire the user's `ask-received` hook.
    pub ask_received_hook: bool,
    /// The channel that answers becomes the active channel.
    pub follow_active_channel: bool,
}

/// The native id of a probe: `setup-probe:<target>:<nonce>`.
pub fn native_id(target: ProbeTarget, nonce: &str) -> String {
    format!("{PREFIX}{}:{nonce}", target.name())
}

/// The probe a request's native id names, or `None` for every other request.
pub fn parse_probe_id(native_request_id: Option<&str>) -> Option<ProbeTarget> {
    let (target, nonce) = native_request_id?.strip_prefix(PREFIX)?.split_once(':')?;
    if nonce.is_empty() {
        return None;
    }
    ProbeTarget::parse(target)
}

/// What the daemon does with a request; `None` is an ordinary request and gets today's behavior.
pub fn policy(target: Option<ProbeTarget>) -> ProbePolicy {
    match target {
        None => ProbePolicy {
            popup: true,
            im: ImScope::All,
            ask_received_hook: true,
            follow_active_channel: true,
        },
        Some(ProbeTarget::Popup) => ProbePolicy {
            popup: true,
            im: ImScope::Nothing,
            ask_received_hook: false,
            follow_active_channel: false,
        },
        Some(ProbeTarget::Channel(id)) => ProbePolicy {
            popup: false,
            im: ImScope::Only(id),
            ask_received_hook: false,
            follow_active_channel: false,
        },
    }
}

/// How a check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The person answered where the question was sent.
    Ok,
    /// Nobody answered in time.
    Timeout,
    /// The person (or the wizard) cancelled it.
    Cancelled,
    /// The question could not be delivered at all.
    Failed,
}

/// Map what the ask returned to an outcome. A cancelled question still exits 0 (its output says
/// so), so the output decides; a non-zero exit means the question never reached a person.
pub fn outcome_of(exit_code: i32, stdout: &str) -> Outcome {
    if exit_code != 0 {
        return Outcome::Failed;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) else {
        return Outcome::Failed;
    };
    match value.get("action").and_then(|action| action.as_str()) {
        Some("answer") => Outcome::Ok,
        _ => Outcome::Cancelled,
    }
}

/// The first request that appeared after `baseline` and is an Agent's own question whose Agent
/// is known; its family name. A new request whose Agent is not known yet stays "new", so a later
/// frame (once the daemon has resolved the Agent) can still match it.
pub fn first_new_agent_ask(
    baseline: &HashSet<String>,
    pending: &[PendingRequestInfo],
) -> Option<String> {
    pending
        .iter()
        .filter(|request| !baseline.contains(&request.id) && request.agent_ask)
        .find_map(|request| request.agent_kind.clone())
}

/// The ids of every request in a frame (the baseline an observation starts from).
pub fn ids_of(pending: &[PendingRequestInfo]) -> HashSet<String> {
    pending.iter().map(|request| request.id.clone()).collect()
}

/// The question a popup or channel check asks: one question, one option to confirm.
pub fn build_task(target: ProbeTarget, nonce: &str, lang: Lang) -> TaskRequest {
    let (explanation, question, option, source) = match (target, lang) {
        (ProbeTarget::Popup, Lang::Zh) => (
            "这是 AskHuman 设置向导发来的测试提问。看到这个窗口，说明本机弹窗工作正常。",
            "能看到这条测试提问吗？",
            "收到",
            "AskHuman 设置向导",
        ),
        (ProbeTarget::Popup, Lang::En) => (
            "This is a test question from the AskHuman setup wizard. Seeing this window means the desktop popup works.",
            "Can you see this test question?",
            "Received",
            "AskHuman setup wizard",
        ),
        (ProbeTarget::Channel(_), Lang::Zh) => (
            "这是 AskHuman 设置向导发来的测试提问。在这里点击「收到」，说明这个渠道既能收到提问、也能把回答送回来。",
            "能看到这条测试提问吗？",
            "收到",
            "AskHuman 设置向导",
        ),
        (ProbeTarget::Channel(_), Lang::En) => (
            "This is a test question from the AskHuman setup wizard. Tap “Received” here to confirm this channel both delivers the question and sends your answer back.",
            "Can you see this test question?",
            "Received",
            "AskHuman setup wizard",
        ),
    };
    TaskRequest {
        caller_cwd: None,
        native_request_id: Some(native_id(target, nonce)),
        message: MessagePrompt::new(explanation.to_string(), Vec::new()),
        questions: vec![Question::new(
            question.to_string(),
            vec![OptionItem::new(option, true)],
        )],
        is_markdown: false,
        source: source.to_string(),
        lang: lang.code().to_string(),
        project: String::new(),
        select_only: true,
        single: true,
        output_format: OutputFormat::Json,
        // The daemon keeps a probe out of the reply history itself; leaving this true keeps the
        // request an ordinary question for the popup (a request without history is a Stop card).
        record_history: true,
        stop_confirmation: false,
        agent_kind: None,
        agent_session_id: None,
        mcp_instance_id: None,
        agent_pid: None,
        // 0 = no Agent to look for in the process tree.
        caller_pid: 0,
        from_mcp: false,
        perf_id: String::new(),
        perf_autodismiss: false,
        whats_next: false,
        escalate: false,
        escalation_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b6d5e4a-1f3c-4d5e-9a7b-2c8d1e0f3a4b";

    fn pending(id: &str, kind: Option<&str>, agent_ask: bool) -> PendingRequestInfo {
        PendingRequestInfo {
            id: id.to_string(),
            preview: String::new(),
            escalated: false,
            agent_kind: kind.map(str::to_string),
            agent_ask,
        }
    }

    #[test]
    fn a_probe_id_round_trips_for_the_popup_and_every_channel() {
        let mut targets = vec![ProbeTarget::Popup];
        targets.extend(channels::ORDER.iter().copied().map(ProbeTarget::Channel));
        for target in targets {
            let id = native_id(target, ID);
            assert!(id.starts_with(PREFIX));
            assert_eq!(parse_probe_id(Some(&id)), Some(target), "{id}");
        }
    }

    #[test]
    fn the_id_names_the_target_and_the_nonce() {
        assert_eq!(
            native_id(ProbeTarget::Channel("slack"), "n1"),
            "setup-probe:slack:n1"
        );
        assert_eq!(native_id(ProbeTarget::Popup, "n1"), "setup-probe:popup:n1");
    }

    #[test]
    fn only_a_well_formed_probe_id_is_a_probe() {
        for not_a_probe in [
            None,
            Some(""),
            Some("setup-probe:"),
            Some("setup-probe:popup"),
            Some("setup-probe:popup:"),
            Some("setup-probe::abc"),
            Some("setup-probe:ios:abc"),
            Some("setup-probe:everything:abc"),
            Some("setup-probe:Popup:abc"),
            Some("Setup-Probe:popup:abc"),
            Some("agent-failure:popup:abc"),
            Some("popup:setup-probe:abc"),
            Some("019e4c1a-codex-native-question"),
        ] {
            assert_eq!(parse_probe_id(not_a_probe), None, "{not_a_probe:?}");
        }
    }

    #[test]
    fn an_ordinary_request_keeps_todays_behavior_in_every_respect() {
        assert_eq!(
            policy(None),
            ProbePolicy {
                popup: true,
                im: ImScope::All,
                ask_received_hook: true,
                follow_active_channel: true,
            }
        );
        assert_eq!(policy(None).im.only(), None);
    }

    #[test]
    fn a_popup_probe_reaches_the_popup_and_no_channel() {
        assert_eq!(
            policy(Some(ProbeTarget::Popup)),
            ProbePolicy {
                popup: true,
                im: ImScope::Nothing,
                ask_received_hook: false,
                follow_active_channel: false,
            }
        );
    }

    #[test]
    fn a_channel_probe_reaches_that_channel_and_nothing_else() {
        let policy = policy(Some(ProbeTarget::Channel("telegram")));
        assert!(!policy.popup);
        assert_eq!(policy.im, ImScope::Only("telegram"));
        assert_eq!(policy.im.only(), Some("telegram"));
        assert!(!policy.ask_received_hook);
        assert!(!policy.follow_active_channel);
    }

    #[test]
    fn an_answer_is_ok_and_a_cancel_is_not() {
        assert_eq!(
            outcome_of(0, r#"{"action":"answer","channel":"popup","answers":[]}"#),
            Outcome::Ok
        );
        assert_eq!(
            outcome_of(0, r#"{"action":"cancel","channel":"popup","status":"x"}"#),
            Outcome::Cancelled
        );
    }

    #[test]
    fn a_question_that_reached_nobody_failed() {
        assert_eq!(outcome_of(crate::app::EXIT_NO_CHANNEL, ""), Outcome::Failed);
        assert_eq!(outcome_of(1, r#"{"action":"answer"}"#), Outcome::Failed);
        // A zero exit with output that is not the expected JSON is no answer either.
        assert_eq!(outcome_of(0, "not json"), Outcome::Failed);
        assert_eq!(outcome_of(0, ""), Outcome::Failed);
    }

    #[test]
    fn outcomes_serialize_in_lowercase() {
        for (outcome, text) in [
            (Outcome::Ok, "ok"),
            (Outcome::Timeout, "timeout"),
            (Outcome::Cancelled, "cancelled"),
            (Outcome::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(outcome).unwrap(), text);
        }
    }

    #[test]
    fn a_request_that_was_already_pending_is_not_the_agents_first_question() {
        let baseline = ids_of(&[pending("old", Some("claude"), true)]);
        assert_eq!(
            first_new_agent_ask(&baseline, &[pending("old", Some("claude"), true)]),
            None
        );
    }

    #[test]
    fn a_new_agent_question_names_its_agent() {
        let baseline = ids_of(&[pending("old", Some("cursor"), true)]);
        let frame = [
            pending("old", Some("cursor"), true),
            pending("new", Some("claude"), true),
        ];
        assert_eq!(
            first_new_agent_ask(&baseline, &frame),
            Some("claude".to_string())
        );
    }

    #[test]
    fn a_new_question_that_is_not_an_agents_does_not_count() {
        let baseline = HashSet::new();
        // A person at a terminal (no Agent), a Stop card, a native or relay question: not an
        // Agent's own question, or the Agent is not known.
        for request in [
            pending("a", None, true),
            pending("b", Some("codex"), false),
            pending("c", None, false),
        ] {
            assert_eq!(first_new_agent_ask(&baseline, &[request]), None);
        }
    }

    #[test]
    fn a_question_whose_agent_is_resolved_later_still_counts_in_a_later_frame() {
        let baseline = HashSet::new();
        assert_eq!(
            first_new_agent_ask(&baseline, &[pending("late", None, true)]),
            None
        );
        assert_eq!(
            first_new_agent_ask(&baseline, &[pending("late", Some("codex"), true)]),
            Some("codex".to_string())
        );
    }

    #[test]
    fn the_first_new_agent_question_wins() {
        let frame = [
            pending("a", Some("pi"), true),
            pending("b", Some("grok"), true),
        ];
        assert_eq!(
            first_new_agent_ask(&HashSet::new(), &frame),
            Some("pi".to_string())
        );
    }

    #[test]
    fn the_question_is_one_confirmation_marked_as_a_probe() {
        for lang in [Lang::En, Lang::Zh] {
            for target in [ProbeTarget::Popup, ProbeTarget::Channel("feishu")] {
                let task = build_task(target, ID, lang);
                assert_eq!(
                    parse_probe_id(task.native_request_id.as_deref()),
                    Some(target)
                );
                assert_eq!(task.questions.len(), 1);
                assert_eq!(task.questions[0].predefined_options.len(), 1);
                assert!(task.select_only && task.single);
                assert_eq!(task.output_format, OutputFormat::Json);
                assert_eq!(task.lang, lang.code());
                assert!(!task.message.text.is_empty());
                assert!(!task.source.is_empty());
            }
        }
    }

    #[test]
    fn the_question_names_no_agent_and_stays_an_ordinary_popup_question() {
        let task = build_task(ProbeTarget::Popup, ID, Lang::En);
        assert_eq!(task.caller_pid, 0);
        assert!(task.agent_kind.is_none() && task.agent_session_id.is_none());
        assert!(!task.from_mcp && !task.whats_next && !task.escalate);
        assert!(task.escalation_reason.is_none());
        // `popup_kind` reads `record_history`: false would turn the question into a Stop card.
        assert!(task.record_history);
        assert_eq!(task.popup_kind(), crate::ipc::PopupRequestKind::Ask);
    }

    #[test]
    fn the_wording_follows_the_language() {
        let en = build_task(ProbeTarget::Popup, ID, Lang::En);
        let zh = build_task(ProbeTarget::Popup, ID, Lang::Zh);
        assert_eq!(en.questions[0].predefined_options[0].text, "Received");
        assert_eq!(zh.questions[0].predefined_options[0].text, "收到");
        assert_ne!(en.message.text, zh.message.text);
        // The channel wording is about the round trip, the popup wording about the window.
        let channel = build_task(ProbeTarget::Channel("slack"), ID, Lang::En);
        assert_ne!(channel.message.text, en.message.text);
    }
}
