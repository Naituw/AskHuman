//! The remote-answering channels the setup wizard offers, in the order it shows them, and their
//! current state. Shared by the GUI wizard (`setup_channel_status`) and the terminal flow.
//!
//! iOS is not part of the wizard (spec D22); popup is the local window and needs no setup.

use crate::cli::channel_cmd;
use crate::config::AppConfig;
use crate::i18n::Lang;
use serde::Serialize;

/// Telegram, Slack, DingTalk, Feishu — the wizard's own order (not `channel_cmd::CHANNELS`).
pub const ORDER: [&str; 4] = ["telegram", "slack", "dingding", "feishu"];

/// Display name of a channel in the given language.
pub fn title(id: &str, lang: Lang) -> &'static str {
    match (id, lang) {
        ("telegram", _) => "Telegram",
        ("slack", _) => "Slack",
        ("dingding", Lang::Zh) => "钉钉",
        ("dingding", Lang::En) => "DingTalk",
        ("feishu", Lang::Zh) => "飞书",
        ("feishu", Lang::En) => "Feishu",
        _ => "",
    }
}

/// The channels whose credentials include a user id the wizard can detect for the user (Telegram
/// takes its chat id by hand).
pub fn can_detect_user_id(id: &str) -> bool {
    matches!(id, "dingding" | "feishu" | "slack")
}

/// Whether the channel's user id (Feishu: open id) is still empty in `config`.
pub fn user_id_missing(config: &AppConfig, id: &str) -> bool {
    let channels = &config.channels;
    let value = match id {
        "dingding" => &channels.dingding.user_id,
        "feishu" => &channels.feishu.open_id,
        "slack" => &channels.slack.user_id,
        _ => return false,
    };
    value.trim().is_empty()
}

/// One channel's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub id: String,
    pub enabled: bool,
    pub configured: bool,
    /// Whether the daemon currently holds a live connection; `None` when the daemon is not
    /// running (unknown, as opposed to "not connected").
    pub connected: Option<bool>,
}

/// Every wizard channel, in order. `connections` is the daemon's `im_connections` list, or `None`
/// when no daemon answered. Reads the keychain to decide `configured`, like `channel list`.
pub fn status(config: &AppConfig, connections: Option<&[String]>) -> Vec<ChannelStatus> {
    status_with(config, connections, &channel_cmd::is_configured)
}

/// [`status`] with the "all required fields and secrets present" check supplied, so tests never
/// read the keychain.
fn status_with(
    config: &AppConfig,
    connections: Option<&[String]>,
    configured: &dyn Fn(&AppConfig, &str) -> bool,
) -> Vec<ChannelStatus> {
    ORDER
        .iter()
        .map(|&id| ChannelStatus {
            id: id.to_string(),
            enabled: channel_cmd::is_enabled(config, id),
            configured: configured(config, id),
            connected: connections
                .map(|names| names.iter().any(|name| name == channel_cmd::conn_name(id))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_the_wizards_own_and_covers_the_four_im_channels() {
        assert_eq!(ORDER, ["telegram", "slack", "dingding", "feishu"]);
        let mut sorted = ORDER.to_vec();
        sorted.sort_unstable();
        let mut cli = channel_cmd::CHANNELS.to_vec();
        cli.sort_unstable();
        assert_eq!(
            sorted, cli,
            "same channels as `channel`, only the order differs"
        );
    }

    #[test]
    fn every_channel_has_a_title_in_both_languages() {
        for id in ORDER {
            assert!(!title(id, Lang::En).is_empty(), "{id}");
            assert!(!title(id, Lang::Zh).is_empty(), "{id}");
        }
        assert_eq!(title("dingding", Lang::Zh), "钉钉");
        assert_eq!(title("dingding", Lang::En), "DingTalk");
    }

    #[test]
    fn user_id_detection_covers_exactly_the_channels_that_can_detect() {
        let config = AppConfig::default();
        for id in ORDER {
            if can_detect_user_id(id) {
                assert!(
                    user_id_missing(&config, id),
                    "{id} starts without a user id"
                );
            } else {
                assert!(!user_id_missing(&config, id), "{id} has nothing to detect");
            }
        }
    }

    #[test]
    fn a_filled_user_id_is_not_missing() {
        let mut config = AppConfig::default();
        config.channels.feishu.open_id = "ou_1".into();
        config.channels.slack.user_id = "   ".into();
        assert!(!user_id_missing(&config, "feishu"));
        assert!(user_id_missing(&config, "slack"), "blank counts as missing");
    }

    fn nothing_is_configured(_: &AppConfig, _: &str) -> bool {
        false
    }

    #[test]
    fn status_marks_connections_unknown_without_a_daemon() {
        let config = AppConfig::default();
        let down = status_with(&config, None, &nothing_is_configured);
        assert!(down.iter().all(|row| row.connected.is_none()));

        let up = status_with(
            &config,
            Some(&["dingtalk".to_string()]),
            &nothing_is_configured,
        );
        let connected: Vec<(&str, Option<bool>)> = up
            .iter()
            .map(|row| (row.id.as_str(), row.connected))
            .collect();
        assert_eq!(
            connected,
            [
                ("telegram", Some(false)),
                ("slack", Some(false)),
                ("dingding", Some(true)),
                ("feishu", Some(false)),
            ]
        );
    }

    #[test]
    fn status_reports_enabled_from_the_config_and_configured_from_the_check() {
        let mut config = AppConfig::default();
        config.channels.slack.enabled = true;
        let rows = status_with(&config, None, &|_, id| id == "feishu");
        let flags: Vec<(&str, bool, bool)> = rows
            .iter()
            .map(|row| (row.id.as_str(), row.enabled, row.configured))
            .collect();
        assert_eq!(
            flags,
            [
                ("telegram", false, false),
                ("slack", true, false),
                ("dingding", false, false),
                ("feishu", false, true),
            ]
        );
    }
}
