//! 核心数据模型（与前端 TS 类型一一对应，serde 采用 camelCase）。

use serde::{Deserialize, Serialize};

/// 一次提问请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub id: String,
    pub message: String,
    pub predefined_options: Vec<String>,
    pub is_markdown: bool,
    /// 提问附带的文件（AI→人，仅用于弹窗展示，不进入结果输出）。
    #[serde(default)]
    pub files: Vec<FileAttachment>,
}

impl AskRequest {
    pub fn new(
        message: String,
        predefined_options: Vec<String>,
        is_markdown: bool,
        files: Vec<FileAttachment>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message,
            predefined_options,
            is_markdown,
            files,
        }
    }
}

/// 提问附带的文件附件（展示用）。`path` 为绝对路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAttachment {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_image: bool,
}

/// 图片附件。`data` 为 base64，可带 `data:...;base64,` 前缀（落盘时清理）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
    #[serde(default)]
    pub filename: Option<String>,
}

/// Channel 的终态动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelAction {
    Send,
    Cancel,
}

/// 某个 Channel 给出的最终回答。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelResult {
    pub action: ChannelAction,
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default)]
    pub user_input: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageAttachment>,
    pub source_channel_id: String,
}

impl ChannelResult {
    pub fn cancel(source_channel_id: impl Into<String>) -> Self {
        Self {
            action: ChannelAction::Cancel,
            selected_options: Vec::new(),
            user_input: None,
            images: Vec::new(),
            source_channel_id: source_channel_id.into(),
        }
    }
}
