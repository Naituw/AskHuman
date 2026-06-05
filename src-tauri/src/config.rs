//! 应用配置：`~/.humaninloop/config.json` 读写、默认值、容错解码。

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

/// 弹窗出现动画样式（对应 macOS `NSWindowAnimationBehavior`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PopupAnimation {
    /// 无动画（NSWindowAnimationBehaviorNone = 2）。
    None,
    /// 文档窗口动画（NSWindowAnimationBehaviorDocumentWindow = 3）。
    Document,
    /// 提示面板动画（NSWindowAnimationBehaviorAlertPanel = 5），更明显。
    #[default]
    Alert,
}

impl PopupAnimation {
    /// 映射到 macOS `NSWindowAnimationBehavior` 原始取值。
    #[cfg(target_os = "macos")]
    pub fn ns_animation_behavior(self) -> isize {
        match self {
            PopupAnimation::None => 2,
            PopupAnimation::Document => 3,
            PopupAnimation::Alert => 5,
        }
    }
}

/// 弹窗背景效果。仅 macOS 26+ 可在二者间切换；旧系统无论选哪个都呈现模糊。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WindowEffect {
    /// macOS 26+ Liquid Glass（`NSGlassEffectView`，由插件应用）。
    #[default]
    Glass,
    /// 传统毛玻璃模糊（`NSVisualEffectView` / UnderWindowBackground）。
    Blur,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralConfig {
    pub theme: ThemeMode,
    pub always_on_top: bool,
    pub appear_animation: PopupAnimation,
    pub window_effect: WindowEffect,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            always_on_top: true,
            appear_animation: PopupAnimation::Alert,
            window_effect: WindowEffect::Glass,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PopupChannelConfig {
    pub enabled: bool,
    pub width: f64,
    pub height: f64,
    pub remember_size: bool,
}

impl Default for PopupChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 560.0,
            height: 620.0,
            remember_size: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramChannelConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub chat_id: String,
    pub api_base_url: String,
}

impl Default for TelegramChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            chat_id: String::new(),
            api_base_url: "https://api.telegram.org".to_string(),
        }
    }
}

/// 钉钉渠道配置。robotCode 不单独配置——企业内部应用机器人 robotCode = clientId(AppKey)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DingTalkChannelConfig {
    pub enabled: bool,
    /// 企业内部应用 AppKey（同时用作机器人 robotCode）。
    pub client_id: String,
    /// 企业内部应用 AppSecret。
    pub client_secret: String,
    /// 接收/作答用户的 userId（单聊）。
    pub user_id: String,
}

impl Default for DingTalkChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            user_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelsConfig {
    pub popup: PopupChannelConfig,
    pub telegram: TelegramChannelConfig,
    pub dingding: DingTalkChannelConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub channels: ChannelsConfig,
}

impl AppConfig {
    /// 从指定路径读取；文件缺失或损坏时返回默认配置（容错）。
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// 原子写入指定路径（临时文件 + rename）。
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 读取默认位置 `~/.humaninloop/config.json`。
    pub fn load() -> Self {
        Self::load_from(&paths::config_file())
    }

    /// 写入默认位置。
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&paths::config_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_are_correct() {
        let c = AppConfig::default();
        assert_eq!(c.general.theme, ThemeMode::System);
        assert!(c.general.always_on_top);
        assert_eq!(c.general.appear_animation, PopupAnimation::Alert);
        assert_eq!(c.general.window_effect, WindowEffect::Glass);
        assert!(c.channels.popup.enabled);
        assert_eq!(c.channels.popup.width, 560.0);
        assert_eq!(c.channels.popup.height, 620.0);
        assert!(c.channels.popup.remember_size);
        assert!(!c.channels.telegram.enabled);
        assert_eq!(c.channels.telegram.api_base_url, "https://api.telegram.org");
        assert!(!c.channels.dingding.enabled);
        assert!(c.channels.dingding.client_id.is_empty());
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let c = AppConfig::load_from(&path);
        assert!(c.general.always_on_top);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"general":{"theme":"dark"}}"#).unwrap();
        let c = AppConfig::load_from(&path);
        assert_eq!(c.general.theme, ThemeMode::Dark);
        // 缺失字段走默认
        assert!(c.general.always_on_top);
        assert!(c.channels.popup.enabled);
    }

    #[test]
    fn unknown_fields_ignored() {
        // 旧版 markdownRenderer 字段应被忽略而非报错
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"general":{"markdownRenderer":"webview","theme":"light"}}"#,
        )
        .unwrap();
        let c = AppConfig::load_from(&path);
        assert_eq!(c.general.theme, ThemeMode::Light);
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut c = AppConfig::default();
        c.general.theme = ThemeMode::Dark;
        c.channels.telegram.enabled = true;
        c.channels.telegram.chat_id = "12345".to_string();
        c.save_to(&path).unwrap();
        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.general.theme, ThemeMode::Dark);
        assert!(loaded.channels.telegram.enabled);
        assert_eq!(loaded.channels.telegram.chat_id, "12345");
    }
}
