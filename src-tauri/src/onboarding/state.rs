//! "Already decided" state of the setup wizard's three domains (`docs/specs/onboarding.md` §4).
//!
//! Almost everything is derived from the machine's current state; only the user's explicit
//! decisions (`skipped`, `background_decided`) are persisted, in `ui-state.json`.
//!
//! - **Agents** (required step; drives the Settings banner, the tray item and the terminal
//!   welcome line): decided when any Agent is integrated, the step was skipped, or the history
//!   holds a reply that came from a real Agent.
//! - **Remote** (optional; drives only the popup footer strip that replaced the old IM tip):
//!   decided when any IM channel is enabled, the step was skipped, or the legacy
//!   `im_tip_dismissed` flag is set. Identical to the old tip's condition.
//! - **Background** (optional; drives no prompt): decided when the GUI Host login item exists or
//!   the user made a choice. It only gates whether `menuBarIcon = always` may write the login item.

use crate::config::AppConfig;
use crate::integrations::agent_mode::Mode;
use crate::integrations::login_item;
use crate::onboarding::agents;
use crate::uistate;

pub const DOMAIN_AGENTS: &str = "agents";
pub const DOMAIN_REMOTE: &str = "remote";
/// The wizard's offer to start Agent tasks from the phone or IM. Not one of the three decided
/// domains: declining it only keeps a rerun of the wizard from offering it again.
pub const DOMAIN_PHONE_TASKS: &str = "phoneTasks";

/// Everything the three "decided" flags are derived from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    pub skipped: Vec<String>,
    /// The retired popup IM tip was dismissed (`ui-state.json` `imTipDismissed`).
    pub legacy_im_tip_dismissed: bool,
    pub any_agent_integrated: bool,
    /// The reply history holds an entry whose `agent_kind` is set.
    pub history_has_agent_origin: bool,
    /// Telegram, DingTalk, Feishu or Slack is enabled (popup and iOS do not count).
    pub any_channel_enabled: bool,
    pub login_item_present: bool,
    pub background_decided: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Decided {
    pub agents: bool,
    pub remote: bool,
    pub background: bool,
}

fn skipped(evidence: &Evidence, domain: &str) -> bool {
    evidence.skipped.iter().any(|d| d == domain)
}

/// Pure derivation of the three flags.
pub fn derive(evidence: &Evidence) -> Decided {
    Decided {
        agents: evidence.any_agent_integrated
            || skipped(evidence, DOMAIN_AGENTS)
            || evidence.history_has_agent_origin,
        remote: evidence.any_channel_enabled
            || skipped(evidence, DOMAIN_REMOTE)
            || evidence.legacy_im_tip_dismissed,
        background: evidence.login_item_present || evidence.background_decided,
    }
}

/// Whether any Agent is currently integrated (CLI or MCP mode). A row whose status cannot be read
/// (Antigravity's strict read) counts as not integrated and never blocks the check.
fn any_agent_integrated() -> bool {
    agents::available()
        .iter()
        .any(|def| matches!(agents::current_mode(def), Ok(Mode::Cli | Mode::Mcp)))
}

/// Whether any IM channel the wizard configures is enabled.
pub fn any_channel_enabled(config: &AppConfig) -> bool {
    let channels = &config.channels;
    channels.telegram.enabled
        || channels.dingding.enabled
        || channels.feishu.enabled
        || channels.slack.enabled
}

/// Read the real machine. Does not touch the keychain.
pub fn gather() -> Evidence {
    let ui = uistate::load();
    Evidence {
        skipped: ui.onboarding.skipped,
        legacy_im_tip_dismissed: ui.im_tip_dismissed,
        any_agent_integrated: any_agent_integrated(),
        history_has_agent_origin: crate::history::has_agent_origin(),
        any_channel_enabled: any_channel_enabled(&AppConfig::load_without_secrets()),
        login_item_present: login_item::instance_has_login_item(),
        background_decided: ui.onboarding.background_decided,
    }
}

pub fn decided() -> Decided {
    derive(&gather())
}

/// `decided().agents`, short-circuited from the cheapest evidence to the dearest.
pub fn agents_decided() -> bool {
    if uistate::load().onboarding.is_skipped(DOMAIN_AGENTS) {
        return true;
    }
    any_agent_integrated() || crate::history::has_agent_origin()
}

/// `decided().remote`.
pub fn remote_decided() -> bool {
    let ui = uistate::load();
    ui.onboarding.is_skipped(DOMAIN_REMOTE)
        || ui.im_tip_dismissed
        || any_channel_enabled(&AppConfig::load_without_secrets())
}

/// Whether the user turned the wizard's phone-task offer down ("Not now").
pub fn phone_tasks_declined() -> bool {
    uistate::load().onboarding.is_skipped(DOMAIN_PHONE_TASKS)
}

/// Record that the user skipped or dismissed a domain. Only `"agents"`, `"remote"` and
/// `"phoneTasks"` are accepted; idempotent.
pub fn mark_skipped(domain: &str) {
    if !uistate::SKIPPABLE_DOMAINS.contains(&domain) {
        return;
    }
    uistate::update(|state| {
        if !state.onboarding.is_skipped(domain) {
            state.onboarding.skipped.push(domain.to_string());
        }
    });
}

/// Record that the user made a choice about the menu-bar / login-item behaviour.
pub fn mark_background_decided() {
    uistate::update(|state| state.onboarding.background_decided = true);
}

/// Whether `menuBarIcon = always` may write the GUI Host login item: an existing login item (the
/// user already lives with it) or an explicit choice. A fresh install stays silent until then.
pub fn login_item_gate_open() -> bool {
    login_item::instance_has_login_item() || uistate::load().onboarding.background_decided
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev() -> Evidence {
        Evidence::default()
    }

    #[test]
    fn nothing_is_decided_on_a_fresh_machine() {
        assert_eq!(derive(&ev()), Decided::default());
    }

    #[test]
    fn each_agent_evidence_decides_only_the_agent_domain() {
        for evidence in [
            Evidence {
                any_agent_integrated: true,
                ..ev()
            },
            Evidence {
                skipped: vec!["agents".to_string()],
                ..ev()
            },
            Evidence {
                history_has_agent_origin: true,
                ..ev()
            },
        ] {
            assert_eq!(
                derive(&evidence),
                Decided {
                    agents: true,
                    remote: false,
                    background: false
                },
                "{evidence:?}"
            );
        }
    }

    #[test]
    fn each_remote_evidence_decides_only_the_remote_domain() {
        for evidence in [
            Evidence {
                any_channel_enabled: true,
                ..ev()
            },
            Evidence {
                skipped: vec!["remote".to_string()],
                ..ev()
            },
            Evidence {
                legacy_im_tip_dismissed: true,
                ..ev()
            },
        ] {
            assert_eq!(
                derive(&evidence),
                Decided {
                    agents: false,
                    remote: true,
                    background: false
                },
                "{evidence:?}"
            );
        }
    }

    #[test]
    fn each_background_evidence_decides_only_the_background_domain() {
        for evidence in [
            Evidence {
                login_item_present: true,
                ..ev()
            },
            Evidence {
                background_decided: true,
                ..ev()
            },
        ] {
            assert_eq!(
                derive(&evidence),
                Decided {
                    agents: false,
                    remote: false,
                    background: true
                },
                "{evidence:?}"
            );
        }
    }

    #[test]
    fn skipping_one_domain_does_not_decide_the_other() {
        let evidence = Evidence {
            skipped: vec!["agents".to_string()],
            ..ev()
        };
        assert!(derive(&evidence).agents);
        assert!(!derive(&evidence).remote);
    }

    #[test]
    fn declining_the_phone_task_offer_decides_none_of_the_domains() {
        let evidence = Evidence {
            skipped: vec![DOMAIN_PHONE_TASKS.to_string()],
            ..ev()
        };
        assert_eq!(derive(&evidence), Decided::default());
    }

    #[test]
    fn channel_domain_ignores_agent_state() {
        // A user with integrated Agents and a manual-only history still sees the remote hint
        // until a channel is enabled or the hint is dismissed.
        let evidence = Evidence {
            any_agent_integrated: true,
            history_has_agent_origin: true,
            ..ev()
        };
        assert!(!derive(&evidence).remote);
    }

    #[test]
    fn channel_counting_excludes_popup_and_ios() {
        let mut config = AppConfig::default();
        assert!(!any_channel_enabled(&config));
        config.channels.popup.enabled = true;
        config.channels.ios.enabled = true;
        assert!(!any_channel_enabled(&config), "popup and iOS do not count");
        config.channels.slack.enabled = true;
        assert!(any_channel_enabled(&config));
    }

    #[cfg(unix)]
    #[test]
    fn gather_reads_the_machine_in_an_isolated_home() {
        use crate::onboarding::testutil;
        if !testutil::in_child() {
            testutil::run_in_child(
                module_path!(),
                "gather_reads_the_machine_in_an_isolated_home",
            );
            return;
        }
        // Fresh machine: nothing decided.
        let fresh = gather();
        assert_eq!(derive(&fresh), Decided::default(), "{fresh:?}");
        assert!(!agents_decided());
        assert!(!remote_decided());
        assert!(!login_item_gate_open());

        // Skipping a domain is remembered and only that domain flips.
        mark_skipped("agents");
        mark_skipped("agents");
        mark_skipped("bogus");
        assert!(agents_decided());
        assert!(!remote_decided());
        assert_eq!(
            uistate::load().onboarding.skipped,
            vec!["agents".to_string()]
        );

        mark_skipped("remote");
        assert!(remote_decided());

        // Turning the phone-task offer down is remembered too, and decides none of the domains.
        assert!(!phone_tasks_declined());
        mark_skipped("phoneTasks");
        mark_skipped("phoneTasks");
        assert!(phone_tasks_declined());
        assert_eq!(
            uistate::load().onboarding.skipped,
            vec![
                "agents".to_string(),
                "remote".to_string(),
                "phoneTasks".to_string()
            ]
        );

        // A choice about the background opens the login-item gate.
        mark_background_decided();
        assert!(login_item_gate_open());
        assert!(derive(&gather()).background);

        // A history entry from a real Agent decides the Agent domain on its own.
        let state_dir = crate::paths::config_dir();
        std::fs::write(state_dir.join("ui-state.json"), b"{}").unwrap();
        assert!(!agents_decided());
        let line = serde_json::json!({
            "id": "a", "timestampMs": 1, "project": "/p", "source": "x",
            "agentKind": "codex", "channel": "popup", "action": "send",
        });
        std::fs::write(crate::paths::history_file(), format!("{line}\n")).unwrap();
        assert!(agents_decided());

        // The legacy tip flag counts as "remote skipped".
        std::fs::write(
            state_dir.join("ui-state.json"),
            br#"{"imTipDismissed": true}"#,
        )
        .unwrap();
        assert!(remote_decided());
        assert!(
            !uistate::load().onboarding.is_skipped("remote"),
            "never rewritten"
        );
    }
}
