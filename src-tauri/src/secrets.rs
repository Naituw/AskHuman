//! OS keychain wrapper for channel secrets.
//!
//! The three channel secrets (DingTalk/Feishu AppSecret, Telegram bot token) are stored in the
//! platform secret store — macOS login keychain / Windows Credential Manager / Linux Secret
//! Service — instead of plaintext in `config.json`. Each secret is a generic password keyed by a
//! fixed service + account.
//!
//! All operations are best-effort. When the store is unreachable (e.g. a headless Linux box with
//! no Secret Service), callers fall back to plaintext config (see `config.rs`). This module is a
//! thin wrapper; the resolve/migrate/fallback policy lives in `config.rs`.

use keyring::{Entry, Error};

/// Keychain service name (matches the app bundle identifier in `tauri.conf.json`).
const SERVICE: &str = "com.naituw.humaninloop";

/// Account keys for each stored secret (stable; used as the keychain item account name).
pub const ACCOUNT_DINGTALK_SECRET: &str = "channels.dingding.clientSecret";
pub const ACCOUNT_FEISHU_SECRET: &str = "channels.feishu.appSecret";
pub const ACCOUNT_TELEGRAM_TOKEN: &str = "channels.telegram.botToken";
/// Slack Bot Token (`xoxb-…`, Web API) and App-Level Token (`xapp-…`, Socket Mode).
pub const ACCOUNT_SLACK_BOT_TOKEN: &str = "channels.slack.botToken";
pub const ACCOUNT_SLACK_APP_TOKEN: &str = "channels.slack.appToken";

/// The secret store could not be reached (e.g. no Secret Service on a headless Linux box).
/// Callers treat this as "use the plaintext config fallback".
#[derive(Debug, Clone, Copy)]
pub struct Unavailable;

/// Test/CI escape hatch: when `ASKHUMAN_NO_KEYCHAIN` is set (non-empty, != "0"), behave as if the
/// secret store were unreachable — reads report `Unavailable`, writes/deletes no-op as `Unavailable`.
/// Callers then fall back to plaintext config (see `config.rs`) without ever touching the real OS
/// keychain (which is NOT isolated by `$HOME`). This keeps isolated runs — e.g. the perf harness
/// under a throwaway `$HOME` — from reading or clobbering the user's real channel secrets.
///
/// Also forced on for Dev Instance mode (`ASKHUMAN_HOME` set): instance configs must never read
/// or migrate into the user's main keychain (see `docs/specs/dev-instance-parallel.md`).
fn disabled() -> bool {
    if crate::dev_instance::is_dev_instance() {
        return true;
    }
    std::env::var("ASKHUMAN_NO_KEYCHAIN")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

fn entry(account: &str) -> Result<Entry, Unavailable> {
    Entry::new(SERVICE, account).map_err(|_| Unavailable)
}

/// Read a secret. `Ok(None)` means "not set"; `Err(Unavailable)` means the store is unreachable.
pub fn get(account: &str) -> Result<Option<String>, Unavailable> {
    if disabled() {
        return Err(Unavailable);
    }
    match entry(account)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(Error::NoEntry) => Ok(None),
        Err(_) => Err(Unavailable),
    }
}

/// Create or overwrite a secret.
pub fn set(account: &str, value: &str) -> Result<(), Unavailable> {
    if disabled() {
        return Err(Unavailable);
    }
    entry(account)?.set_password(value).map_err(|_| Unavailable)
}

/// Delete a secret. A missing entry counts as success.
pub fn delete(account: &str) -> Result<(), Unavailable> {
    if disabled() {
        return Err(Unavailable);
    }
    match entry(account)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(_) => Err(Unavailable),
    }
}

/// Whether a secret is currently stored (for the settings "Saved" indicator).
/// Returns false when unset or when the store is unreachable.
pub fn has(account: &str) -> bool {
    matches!(get(account), Ok(Some(_)))
}
