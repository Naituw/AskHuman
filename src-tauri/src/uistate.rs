//! 轻量 UI 状态（`~/.askhuman/ui-state.json`）：与用户配置（config.json）分离的界面一次性
//! 标记，如「弹窗 IM 引导提示已关闭」。字段增量演进（`serde(default)`），读失败视为默认值；
//! 写失败静默（丢标记的代价只是提示多显示一次）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiState {
    /// 弹窗页脚「配置 IM 渠道」一次性引导已被关闭（点 ✕ 或点「打开设置」后置位）。
    pub im_tip_dismissed: bool,
}

fn path() -> PathBuf {
    crate::paths::config_dir().join("ui-state.json")
}

pub fn load() -> UiState {
    std::fs::read(path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

pub fn save(state: &UiState) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(&p, data);
    }
}
