//! Prevent a project daemon from opening the formal bot's connection.
use super::{relay, routing, workspace};
use crate::config::AppConfig;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.trim().as_bytes()))
}

fn slack_app(token: &str) -> Option<&str> {
    let mut parts = token.split('-');
    if parts.next()? != "xapp" || parts.next()? != "1" {
        return None;
    }
    let app = parts.next()?;
    (app.starts_with('A') && app.len() > 1 && app.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then_some(app)
}

pub fn identities(config: &AppConfig) -> BTreeMap<String, String> {
    let channels = &config.channels;
    let mut result = BTreeMap::new();
    for (name, enabled, value) in [
        (
            "feishu",
            channels.feishu.enabled,
            channels.feishu.app_id.as_str(),
        ),
        (
            "dingding",
            channels.dingding.enabled,
            channels.dingding.client_id.as_str(),
        ),
        (
            "telegram",
            channels.telegram.enabled,
            channels
                .telegram
                .bot_token
                .split(':')
                .next()
                .unwrap_or_default(),
        ),
        (
            "slack",
            channels.slack.enabled,
            slack_app(&channels.slack.app_token).unwrap_or(&channels.slack.app_token),
        ),
    ] {
        if enabled && !value.trim().is_empty() {
            result.insert(name.into(), digest(value));
        }
    }
    if channels.ios.enabled {
        if let Ok(bytes) = std::fs::read(crate::paths::ios_dir().join("state.json")) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(id) = value["macId"].as_str().filter(|id| !id.is_empty()) {
                    result.insert("ios".into(), digest(id));
                }
            }
        }
    }
    result
}

fn separate(
    test: &BTreeMap<String, String>,
    formal: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (channel, identity) in test {
        if formal.get(channel) == Some(identity) {
            return Err(format!("Dev {channel} uses the formal channel identity; configure a separate test bot or pairing"));
        }
    }
    Ok(())
}

pub fn check(root: &Path, config: &AppConfig) -> Result<(), String> {
    if config.channels.slack.enabled
        && !config.channels.slack.app_token.is_empty()
        && slack_app(&config.channels.slack.app_token).is_none()
    {
        return Err(
            "cannot identify this Slack app token format; no Dev channel connection was opened"
                .into(),
        );
    }
    let test = identities(config);
    if test.is_empty() {
        return Ok(());
    }
    let target = relay::system_target_for(root)?;
    let mut command = std::process::Command::new(&target.binary);
    routing::configure(&mut command, &target)?;
    super::clear_client_context(&mut command);
    let mut child = command
        .arg("__dev-channel-identities")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("formal channel identity check timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() || output.stdout.len() > 64 * 1024 {
        return Err("cannot verify separate test channel identities".into());
    }
    let formal = serde_json::from_slice(&output.stdout)
        .map_err(|_| "invalid formal channel identity response")?;
    separate(&test, &formal)
}

/// Called before initial connection and every config reload. Failure closes only Dev channels.
pub fn runtime_config(mut config: AppConfig) -> AppConfig {
    let root = routing::inherited()
        .ok()
        .flatten()
        .filter(|target| target.kind == routing::TargetKind::Dev)
        .and_then(|target| target.worktree)
        .filter(|root| workspace::is_project(root));
    if let Some(root) = root {
        if let Err(error) = check(&root, &config) {
            eprintln!("AskHuman: Dev test channels disabled: {error}");
            config.channels.feishu.enabled = false;
            config.channels.dingding.enabled = false;
            config.channels.telegram.enabled = false;
            config.channels.slack.enabled = false;
            config.channels.ios.enabled = false;
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotating_a_slack_token_does_not_create_a_separate_app() {
        assert_eq!(
            slack_app("xapp-1-A123-first"),
            slack_app("xapp-1-A123-second")
        );
        assert_ne!(
            slack_app("xapp-1-A123-first"),
            slack_app("xapp-1-A456-first")
        );
        assert_eq!(slack_app("unknown-token-format"), None);
    }
    #[test]
    fn different_recipients_do_not_make_the_same_bot_safe() {
        let mut config = AppConfig::default();
        config.channels.telegram.enabled = true;
        config.channels.telegram.bot_token = "123:secret-a".into();
        let formal = identities(&config);
        config.channels.telegram.chat_id = "different recipient".into();
        config.channels.telegram.bot_token = "123:rotated-secret".into();
        assert!(separate(&identities(&config), &formal).is_err());
        config.channels.telegram.bot_token = "456:another-bot".into();
        assert!(separate(&identities(&config), &formal).is_ok());
    }
}
