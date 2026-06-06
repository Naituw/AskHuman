//! 后端国际化：单一配置 `general.language`（auto|en|zh）解析为 `Lang`，
//! 用于 CLI / 窗口标题 / macOS 菜单·Dock / 通知 / 远程渠道等用户可见文案。
//!
//! 源语言为英文；缺失/未知一律回退英文。词条在各里程碑逐步扩充（M4/M5）。

use crate::config::AppConfig;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// 由配置语言字符串解析：显式 en/zh 直用；其余（auto/未知）跟随系统。
    pub fn resolve(cfg_language: &str) -> Lang {
        match cfg_language {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            _ => Lang::from_system(),
        }
    }

    /// 跟随系统：首选语言以 "zh" 开头→中文，否则英文。
    pub fn from_system() -> Lang {
        match sys_locale::get_locale() {
            Some(l) if l.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
            _ => Lang::En,
        }
    }

    /// 读取已保存配置解析当前界面语言。
    pub fn current() -> Lang {
        Lang::resolve(&AppConfig::load().general.language)
    }
}

/// 二选一：按当前语言取英文或中文文案。
#[inline]
fn pick(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

/// 错误行前缀（CLI/stderr）。
pub fn err_prefix(lang: Lang) -> &'static str {
    pick(lang, "Error: ", "错误: ")
}

/// 警告行前缀（CLI/stderr）。
pub fn warn_prefix(lang: Lang) -> &'static str {
    pick(lang, "Warning: ", "警告: ")
}

/// 词条查询：源语言英文，缺失 key 原样返回（兜底）。
/// 含 `{x}` 占位符的模板由调用方用 `str::replace` 填充。
/// `key` 取 `'static`（调用方均传字面量），以便兜底分支可原样返回。
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    match key {
        // —— 结果区块标记（output.rs 实际输出 + agent-help 文档须一致）——
        "marker.options" => pick(lang, "[Selected options]", "[选择的选项]"),
        "marker.input" => pick(lang, "[User input]", "[用户输入]"),
        "marker.images" => pick(lang, "[Images]", "[图片]"),
        "marker.files" => pick(lang, "[Files]", "[文件]"),
        "marker.status" => pick(lang, "[Status]", "[状态]"),

        // —— 结果状态文案 ——
        "status.cancel" => pick(
            lang,
            "The user canceled. You must ask again whether they really want to cancel, and keep asking until they give a clear answer.",
            "用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复",
        ),
        "status.unanswered" => pick(lang, "The user did not answer this question", "用户未回答此问题"),
        "status.confirmContinue" => pick(lang, "User confirmed to continue", "用户确认继续"),

        // —— 窗口标题 ——
        "title.popup" => "HumanInLoop",
        "title.settings" => pick(lang, "HumanInLoop Settings", "HumanInLoop 设置"),

        // —— macOS 附件右键菜单 ——
        "menu.open" => pick(lang, "Open", "打开"),
        "menu.openWith" => pick(lang, "Open With", "打开方式"),
        "menu.other" => pick(lang, "Other…", "其他…"),
        "menu.quickLook" => pick(lang, "Quick Look “{name}”", "快速查看「{name}」"),
        "menu.revealInFinder" => pick(lang, "Show in Finder", "在访达中显示"),
        "menu.copyFile" => pick(lang, "Copy “{name}”", "拷贝「{name}」"),
        "menu.copyPath" => pick(lang, "Copy Path", "拷贝路径"),
        "menu.appFallback" => pick(lang, "App", "应用"),

        // —— CLI 解析/分发错误 ——
        "cli.missingContent" => pick(lang, "missing question content", "缺少提问内容"),
        "cli.unknownOption" => pick(lang, "unknown option {opt}", "未知选项 {opt}"),
        "cli.unknownOptionColon" => pick(lang, "unknown option: {opt}", "未知选项: {opt}"),
        "cli.optionMissingValue" => pick(lang, "{opt} option is missing a value", "{opt} 选项缺少参数值"),
        "cli.optionBeforeQuestion" => pick(
            lang,
            "{opt} cannot appear before the first question (-q)",
            "{opt} 不能出现在第一个问题(-q)之前",
        ),
        "cli.positionalOnlyMessage" => pick(
            lang,
            "a positional argument is only allowed as the Message, and must come first",
            "位置参数只能作为 Message，且需在最前",
        ),

        // —— 文件附件解析错误 ——
        "cli.fileNotFound" => pick(lang, "file not found or inaccessible: {path}", "文件不存在或无法访问: {path}"),
        "cli.notAFile" => pick(lang, "not a file: {path}", "不是文件: {path}"),

        // —— 图片落盘错误 ——
        "cli.createImageDirFailed" => pick(lang, "failed to create image directory: {path}", "创建图片目录失败: {path}"),
        "cli.writeImageFailed" => pick(lang, "failed to write image: {path}", "写入图片失败: {path}"),
        "cli.imageDecodeFailed" => pick(lang, "failed to decode image base64", "图片 base64 解码失败"),

        // —— 运行/渠道可用性（stderr，含自带前缀的整行）——
        "app.popupUnavailableFellBack" => pick(
            lang,
            "Local popup unavailable: {reason}; using messaging channel instead",
            "本地弹窗不可用：{reason}；已改用消息渠道",
        ),
        "app.popupUnavailableNoChannel" => pick(
            lang,
            "Local popup unavailable: {reason}, and no messaging channel is configured",
            "本地弹窗不可用：{reason}，且未配置可用的消息渠道",
        ),
        "app.popupDisabledNoChannel" => pick(
            lang,
            "Local popup is disabled, and no messaging channel is configured",
            "本地弹窗已禁用，且未配置可用的消息渠道",
        ),
        "app.noChannel" => pick(lang, "no available channel — {reason}", "无可用的通信 Channel — {reason}"),
        "app.popupStartFailedFellBack" => pick(
            lang,
            "Local popup failed to start: {e}; using messaging channel instead",
            "本地弹窗启动失败：{e}；已改用消息渠道",
        ),
        "app.popupStartFailedNoChannel" => pick(
            lang,
            "Local popup failed to start: {e}, and no messaging channel is configured",
            "本地弹窗启动失败：{e}，且未配置可用的消息渠道",
        ),
        "app.runtimeCreateFailed" => pick(lang, "failed to create runtime: {e}", "无法创建运行时: {e}"),
        "app.telegramInvalid" => pick(lang, "invalid Telegram config: {e}", "Telegram 配置无效: {e}"),
        "app.dingtalkInvalid" => pick(lang, "invalid DingTalk config: {e}", "钉钉配置无效: {e}"),
        "app.sessionEndedNoResult" => pick(
            lang,
            "messaging session ended without a result",
            "消息渠道会话结束但未获得结果",
        ),
        "app.settingsLaunchFailed" => pick(lang, "failed to launch settings: {e}", "无法启动设置界面: {e}"),
        "app.noDisplay" => pick(
            lang,
            "no graphical display (neither DISPLAY nor WAYLAND_DISPLAY is set)",
            "无图形显示环境（DISPLAY / WAYLAND_DISPLAY 均未设置）",
        ),
        "app.noWebkitgtk" => pick(
            lang,
            "missing WebKitGTK on the system (e.g. libwebkit2gtk-4.1)",
            "系统缺少 WebKitGTK（如 libwebkit2gtk-4.1）",
        ),

        // —— 远程渠道（Telegram / 钉钉）发给用户的文案 ——
        "channel.questionFrom" => pick(lang, "Question from {source}", "来自 {source} 的提问"),
        // Message（共享描述）头部：中英一律英文，{source} 默认 "the Loop"。
        "channel.messageFrom" => pick(lang, "Message from {source}", "Message from {source}"),
        "channel.questionIndexed" => pick(lang, "Question {i}/{n}", "问题 {i}/{n}"),
        "channel.tgSendButton" => pick(lang, "↑ Submit", "↑ 提交"),
        // 抢答收尾：赢家端名称 + 卡片终态状态行。
        "channel.sourcePopup" => pick(lang, "Popup", "弹窗"),
        "channel.sourceTelegram" => pick(lang, "Telegram", "Telegram"),
        "channel.sourceDingTalk" => pick(lang, "DingTalk", "钉钉"),
        "channel.tgReplied" => pick(lang, "✅ Replied", "✅ 已回复"),
        "channel.tgAnsweredVia" => pick(lang, "✅ Answered via {source}", "✅ 已在{source}回答"),
        // 钉钉卡片终态文案（绑定模板变量 submit_status）。卡片自带样式，文案不加 emoji 前缀。
        "channel.ddSubmitted" => pick(lang, "Submitted", "已提交"),
        "channel.ddAnsweredVia" => pick(lang, "Answered via {source}", "已在{source}回答"),
        "channel.tgActionHint" => pick(
            lang,
            "💬 To add more, just send text messages here. Anything you send after this card (before tapping Submit) will be included.",
            "💬 需要补充可直接在这里发文字消息。本卡片发出后、点「提交」前你发送的文字都会一并提交。",
        ),
        "channel.fileSendFailed" => pick(lang, "⚠️ Failed to send file: {name}", "⚠️ 文件发送失败：{name}"),
        "channel.ddTitleFallback" => pick(lang, "Question", "提问"),
        "channel.ddHintFree" => pick(
            lang,
            "👉 Just reply with text; you can also send images / files",
            "👉 直接回复文字即可；也可发送图片 / 文件",
        ),
        "channel.ddHintOptions" => pick(
            lang,
            "👉 Reply with the option number(s) (comma-separated for multiple, e.g. 1,3), or just type your reply; you can also send images / files",
            "👉 回复编号选择（多选用逗号，如 1,3），或直接输入文字；也可发送图片 / 文件",
        ),
        "channel.cardSendButton" => pick(lang, "Send", "发送"),

        // —— 渠道本地诊断（stderr，含 warn/err 前缀由调用方拼接）——
        "channel.tgConfigInvalidSkip" => pick(
            lang,
            "invalid Telegram config, skipping this channel: {e}",
            "Telegram 配置无效，已跳过该 Channel: {e}",
        ),
        "channel.ddConfigInvalidSkip" => pick(
            lang,
            "invalid DingTalk config, skipping this channel: {e}",
            "钉钉配置无效，已跳过该 Channel: {e}",
        ),
        "channel.fileSendFailedLog" => pick(lang, "failed to send file: {path}: {e}", "文件发送失败: {path}: {e}"),
        "channel.ddMessageSendFailed" => pick(lang, "failed to send DingTalk Message: {e}", "钉钉 Message 发送失败: {e}"),
        "channel.ddFileSendFailedLog" => pick(lang, "failed to send DingTalk file: {path}: {e}", "钉钉文件发送失败: {path}: {e}"),
        "channel.ddQuestionSendFailed" => pick(lang, "failed to send DingTalk question: {e}", "钉钉提问发送失败: {e}"),
        "channel.ddCardDeliverFailed" => pick(lang, "failed to deliver DingTalk card, falling back to text: {e}", "钉钉互动卡片投放失败，回退纯文本: {e}"),
        "channel.ddImageDownloadFailed" => pick(lang, "failed to download DingTalk image: {e}", "钉钉图片下载失败: {e}"),
        "channel.ddFileDownloadFailed" => pick(lang, "failed to download DingTalk file: {e}", "钉钉文件下载失败: {e}"),

        // —— 设置页「弹出测试窗口」示例内容 ——
        "test.message" => pick(
            lang,
            "This is a test popup for previewing the appear animation and appearance.",
            "这是一个测试弹窗，用于预览弹出动画与外观。",
        ),
        "test.question" => pick(lang, "Test question: how does the popup look?", "测试问题：弹窗效果看起来如何？"),
        "test.optionGood" => pick(lang, "Looks good", "很好"),
        "test.optionAdjust" => pick(lang, "Needs tweaks", "再调整"),

        // —— IPC 命令直接返回的 GUI 文案（commands.rs）——
        "cmd.invalidAttachmentIndex" => pick(lang, "Invalid attachment index", "无效的附件索引"),
        "cmd.readFileFailed" => pick(lang, "Failed to read file: {e}", "读取文件失败: {e}"),
        "cmd.fileIconUnsupported" => pick(
            lang,
            "Getting a file icon is not supported on this platform",
            "当前平台不支持获取文件图标",
        ),
        "cmd.openFailed" => pick(lang, "Failed to open: {e}", "打开失败: {e}"),
        "cmd.locateExeFailed" => pick(lang, "Failed to locate the program path: {e}", "无法定位程序路径: {e}"),
        "cmd.testPopupFailed" => pick(lang, "Failed to launch the test popup: {e}", "启动测试弹窗失败: {e}"),
        "cmd.hookInstalled" => pick(lang, "Cursor Hook installed", "已安装 Cursor Hook"),
        "cmd.hookRemoved" => pick(lang, "Cursor Hook removed", "已移除 Cursor Hook"),

        // —— Telegram 测试连接（commands.telegram_test / test_connection）——
        "cmd.tgTestRemote" => pick(
            lang,
            "🤖 HumanInLoop test message\n\nThis is a test message — your Telegram Bot is configured correctly!",
            "🤖 HumanInLoop 测试消息\n\n这是一条测试消息，表示 Telegram Bot 配置成功！",
        ),
        "cmd.tgTestSent" => pick(
            lang,
            "Test message sent! Your Telegram Bot is configured correctly.",
            "测试消息发送成功！Telegram Bot 配置正确。",
        ),

        // —— 钉钉测试连接 / 自动识别（commands.dingtalk_test / dingtalk_detect_*）——
        "cmd.fillUserId" => pick(
            lang,
            "Please fill in UserId first (use “Auto-detect” to get it)",
            "请先填写 UserId（可点击「自动识别」获取）",
        ),
        "cmd.ddTestRemote" => pick(
            lang,
            "✅ HumanInLoop DingTalk connection test succeeded",
            "✅ HumanInLoop 钉钉连接测试成功",
        ),
        "cmd.ddTestSent" => pick(
            lang,
            "A test message was sent to your direct chat — please check DingTalk",
            "已向你的单聊发送一条测试消息，请在钉钉查收",
        ),
        "cmd.fillClientIdSecret" => pick(
            lang,
            "Please fill in ClientId and ClientSecret first",
            "请先填写 ClientId 和 ClientSecret",
        ),
        "cmd.detectCodeInvalid" => pick(lang, "Invalid detect code, please retry", "识别码无效，请重试"),
        "cmd.detectTimeout" => pick(
            lang,
            "Timed out after 120s without a matching detect code, please retry",
            "等待超时（120 秒）未收到匹配的识别码，请重试",
        ),
        "cmd.streamDisconnected" => pick(lang, "Stream disconnected, please retry", "Stream 连接断开，请重试"),

        // —— 错误类型校验文案（Telegram/钉钉 Error::localized）——
        "err.tgEmptyToken" => pick(lang, "Bot Token must not be empty", "Bot Token 不能为空"),
        "err.tgEmptyChatId" => pick(lang, "Chat ID must not be empty", "Chat ID 不能为空"),
        "err.tgInvalidChatId" => pick(
            lang,
            "Invalid Chat ID format; enter a valid numeric ID",
            "Chat ID 格式无效，请输入有效的数字 ID",
        ),
        "err.ddEmptyConfig" => pick(lang, "{field} must not be empty", "{field} 不能为空"),

        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_overrides_system() {
        assert_eq!(Lang::resolve("en"), Lang::En);
        assert_eq!(Lang::resolve("zh"), Lang::Zh);
    }

    #[test]
    fn auto_or_unknown_follows_system() {
        // 仅验证不 panic 且落在二者之一。
        let a = Lang::resolve("auto");
        let b = Lang::resolve("nonsense");
        assert!(matches!(a, Lang::En | Lang::Zh));
        assert!(matches!(b, Lang::En | Lang::Zh));
    }
}
