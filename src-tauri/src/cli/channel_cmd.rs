//! `AskHuman channel <list|set|enable|disable|test|detect|help>` —— IM 渠道配置主入口。
//! 强引导（终端无 flag → 交互向导）+ 可脚本（带 flag → 非交互；密钥仅 env/file/stdin，不进 argv）。
//! 复用 `commands.rs` 的 *_test / *_detect_* 与 `config.rs` 的 load/save（密钥经 save 入钥匙串）。

use super::cfgio::{self, SecretSource};
use crate::config::AppConfig;
use crate::i18n::{err_prefix, Lang};
use std::collections::HashMap;
use std::process::exit;

pub(crate) const CHANNELS: [&str; 4] = ["telegram", "dingding", "feishu", "slack"];

pub fn dispatch(args: &[String], lang: Lang) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let r = match sub {
        "list" | "ls" => list(rest, lang),
        "set" => set(rest, lang),
        "enable" => toggle(rest, true, lang),
        "disable" => toggle(rest, false, lang),
        "test" => test(rest, lang),
        "detect" => detect(rest, lang),
        "help" | "-h" | "--help" => {
            print_line(&help(lang));
            Ok(())
        }
        other => Err(cfgio::t(
            lang,
            &format!("unknown subcommand: {other}\n\n{}", help(lang)),
            &format!("未知子命令: {other}\n\n{}", help(lang)),
        )),
    };
    if let Err(e) = r {
        eprintln!("{}{}", err_prefix(lang), e);
        exit(1);
    }
}

// ——— list ———

fn list(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let cfg = AppConfig::load_without_secrets();
    let status = cfgio::daemon_status();
    let conns = status
        .as_ref()
        .map(|s| s.im_connections.clone())
        .unwrap_or_default();
    let daemon_up = status.is_some();

    if json {
        let arr: Vec<serde_json::Value> = CHANNELS
            .iter()
            .map(|&name| {
                // daemon 未运行时连接状态未知 → null（区别于「确定未连接」）。
                let connected = if daemon_up {
                    serde_json::Value::Bool(conns.contains(&conn_name(name).to_string()))
                } else {
                    serde_json::Value::Null
                };
                serde_json::json!({
                    "name": name,
                    "enabled": is_enabled(&cfg, name),
                    "configured": is_configured(&cfg, name),
                    "connected": connected,
                })
            })
            .collect();
        print_line(&serde_json::to_string_pretty(&serde_json::json!(arr)).unwrap_or_default());
        return Ok(());
    }

    let yes = cfgio::t(lang, "yes", "是");
    let no = cfgio::t(lang, "no", "否");
    let yn = |b: bool| if b { yes.clone() } else { no.clone() };
    print_line(&cfgio::t(
        lang,
        "channel    enabled  configured  connected",
        "渠道        已启用   配置齐全    已连接",
    ));
    for &name in &CHANNELS {
        let connected = if daemon_up {
            yn(conns.contains(&conn_name(name).to_string()))
        } else {
            cfgio::t(lang, "n/a", "—")
        };
        print_line(&format!(
            "{:<10} {:<8} {:<11} {}",
            name,
            yn(is_enabled(&cfg, name)),
            yn(is_configured(&cfg, name)),
            connected
        ));
    }
    if !daemon_up {
        print_line("");
        print_line(&cfgio::t(
            lang,
            "(daemon not running — connection state unknown)",
            "（daemon 未运行 —— 连接状态未知）",
        ));
    }
    Ok(())
}

// ——— set ———

fn set(args: &[String], lang: Lang) -> Result<(), String> {
    let name = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: channel set <name> [flags]",
            "用法: channel set <渠道> [选项]",
        )
    })?;
    let canonical = canon(name, lang)?;
    let rest = &args[1..];
    let pf = parse_flags(rest, lang)?;

    if !pf.has_any() {
        if cfgio::stdin_is_tty() {
            return wizard(canonical, lang);
        }
        return Err(cfgio::t(
            lang,
            "no flags given and not a terminal: pass flags (see 'channel help') or run in a terminal for the wizard",
            "未提供选项且非终端：请传入选项（见 'channel help'）或在终端中运行以使用向导",
        ));
    }
    apply_flags(canonical, pf, lang)
}

/// 解析后的非交互选项：开关 + 非密钥值 + 密钥来源。
#[derive(Default)]
struct ParsedFlags {
    enabled: Option<bool>,
    values: HashMap<String, String>,
    secrets: HashMap<String, SecretSource>,
}

impl ParsedFlags {
    fn has_any(&self) -> bool {
        self.enabled.is_some() || !self.values.is_empty() || !self.secrets.is_empty()
    }
}

fn parse_flags(args: &[String], lang: Lang) -> Result<ParsedFlags, String> {
    let mut pf = ParsedFlags::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--enable" {
            pf.enabled = Some(true);
            i += 1;
        } else if a == "--disable" {
            pf.enabled = Some(false);
            i += 1;
        } else if let Some(name) = a.strip_prefix("--") {
            if let Some(field) = name.strip_suffix("-stdin") {
                pf.secrets.insert(field.to_string(), SecretSource::Stdin);
                i += 1;
            } else if let Some(field) = name.strip_suffix("-env") {
                let v = args.get(i + 1).ok_or_else(|| needs_value(lang, a))?;
                pf.secrets
                    .insert(field.to_string(), SecretSource::Env(v.clone()));
                i += 2;
            } else if let Some(field) = name.strip_suffix("-file") {
                let v = args.get(i + 1).ok_or_else(|| needs_value(lang, a))?;
                pf.secrets
                    .insert(field.to_string(), SecretSource::File(v.clone()));
                i += 2;
            } else {
                let v = args.get(i + 1).ok_or_else(|| needs_value(lang, a))?;
                // 值 "-" 表示从 stdin 读该（密钥）字段。
                if v == "-" {
                    pf.secrets.insert(name.to_string(), SecretSource::Stdin);
                } else {
                    pf.values.insert(name.to_string(), v.clone());
                }
                i += 2;
            }
        } else {
            return Err(cfgio::t(
                lang,
                &format!("unexpected argument: {a}"),
                &format!("非预期参数: {a}"),
            ));
        }
    }
    Ok(pf)
}

fn needs_value(lang: Lang, flag: &str) -> String {
    cfgio::t(
        lang,
        &format!("{flag} needs a value"),
        &format!("{flag} 需要参数值"),
    )
}

fn apply_flags(name: &str, mut pf: ParsedFlags, lang: Lang) -> Result<(), String> {
    let mut cfg = AppConfig::load_without_secrets();
    match name {
        "telegram" => {
            let c = &mut cfg.channels.telegram;
            if let Some(e) = pf.enabled {
                c.enabled = e;
            }
            if let Some(v) = pf.values.remove("chat-id") {
                c.chat_id = v;
            }
            if let Some(v) = pf.values.remove("api-base-url") {
                c.api_base_url = v;
            }
            if let Some(src) = pf.secrets.remove("bot-token") {
                c.bot_token = cfgio::read_secret(&src, lang)?;
            }
        }
        "dingding" => {
            let c = &mut cfg.channels.dingding;
            if let Some(e) = pf.enabled {
                c.enabled = e;
            }
            if let Some(v) = pf.values.remove("client-id") {
                c.client_id = v;
            }
            if let Some(v) = pf.values.remove("user-id") {
                c.user_id = v;
            }
            if let Some(v) = pf.values.remove("card-template-id") {
                c.card_template_id = v;
            }
            if let Some(v) = pf.values.remove("inline-small-text") {
                c.inline_small_text = cfgio::parse_bool(&v)?;
            }
            if let Some(v) = pf.values.remove("convert-text-to-docx") {
                c.convert_text_to_docx = cfgio::parse_bool(&v)?;
            }
            if let Some(src) = pf.secrets.remove("client-secret") {
                c.client_secret = cfgio::read_secret(&src, lang)?;
            }
        }
        "feishu" => {
            let c = &mut cfg.channels.feishu;
            if let Some(e) = pf.enabled {
                c.enabled = e;
            }
            if let Some(v) = pf.values.remove("app-id") {
                c.app_id = v;
            }
            if let Some(v) = pf.values.remove("open-id") {
                c.open_id = v;
            }
            if let Some(v) = pf.values.remove("base-url") {
                c.base_url = v;
            }
            if let Some(src) = pf.secrets.remove("app-secret") {
                c.app_secret = cfgio::read_secret(&src, lang)?;
            }
        }
        "slack" => {
            let c = &mut cfg.channels.slack;
            if let Some(e) = pf.enabled {
                c.enabled = e;
            }
            if let Some(v) = pf.values.remove("user-id") {
                c.user_id = v;
            }
            if let Some(src) = pf.secrets.remove("bot-token") {
                c.bot_token = cfgio::read_secret(&src, lang)?;
            }
            if let Some(src) = pf.secrets.remove("app-token") {
                c.app_token = cfgio::read_secret(&src, lang)?;
            }
        }
        _ => unreachable!(),
    }
    // 残留未识别选项 → 报错（避免静默忽略拼写错误的字段）。
    let mut leftover: Vec<String> = pf.values.keys().cloned().collect();
    leftover.extend(pf.secrets.keys().cloned());
    if !leftover.is_empty() {
        leftover.sort();
        return Err(cfgio::t(
            lang,
            &format!("unknown option(s) for {name}: {}", leftover.join(", ")),
            &format!("{name} 不支持的选项: {}", leftover.join(", ")),
        ));
    }
    cfg.save().map_err(|e| e.to_string())?;
    print_line(&cfgio::t(
        lang,
        &format!("{name} updated"),
        &format!("{name} 已更新"),
    ));
    Ok(())
}

// ——— enable / disable ———

fn toggle(args: &[String], on: bool, lang: Lang) -> Result<(), String> {
    let name = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: channel enable|disable <name>",
            "用法: channel enable|disable <渠道>",
        )
    })?;
    let canonical = canon(name, lang)?;
    let mut cfg = AppConfig::load_without_secrets();
    set_enabled(&mut cfg, canonical, on);
    cfg.save().map_err(|e| e.to_string())?;
    let word = if on {
        cfgio::t(lang, "enabled", "已启用")
    } else {
        cfgio::t(lang, "disabled", "已禁用")
    };
    print_line(&format!("{canonical} {word}"));
    Ok(())
}

// ——— test ———

fn test(args: &[String], lang: Lang) -> Result<(), String> {
    let name = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: channel test <name>",
            "用法: channel test <渠道>",
        )
    })?;
    let canonical = canon(name, lang)?;
    let cfg = AppConfig::load();
    let res: Result<String, String> = cfgio::block_on(async {
        match canonical {
            "telegram" => {
                crate::commands::telegram_test(crate::commands::TelegramTestArgs {
                    bot_token: String::new(),
                    chat_id: cfg.channels.telegram.chat_id.clone(),
                    api_base_url: cfg.channels.telegram.api_base_url.clone(),
                })
                .await
            }
            "dingding" => {
                crate::commands::dingtalk_test(crate::commands::DingTalkTestArgs {
                    client_id: cfg.channels.dingding.client_id.clone(),
                    client_secret: String::new(),
                    user_id: cfg.channels.dingding.user_id.clone(),
                })
                .await
            }
            "feishu" => {
                crate::commands::feishu_test(crate::commands::FeishuTestArgs {
                    app_id: cfg.channels.feishu.app_id.clone(),
                    app_secret: String::new(),
                    open_id: cfg.channels.feishu.open_id.clone(),
                    base_url: cfg.channels.feishu.base_url.clone(),
                })
                .await
            }
            "slack" => {
                crate::commands::slack_test(crate::commands::SlackTestArgs {
                    bot_token: String::new(),
                    app_token: String::new(),
                    user_id: cfg.channels.slack.user_id.clone(),
                })
                .await
            }
            _ => unreachable!(),
        }
    });
    match res {
        Ok(msg) => {
            print_line(&msg);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ——— detect ———

fn detect(args: &[String], lang: Lang) -> Result<(), String> {
    let name = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: channel detect <name>",
            "用法: channel detect <渠道>",
        )
    })?;
    let canonical = canon(name, lang)?;
    let save = args.iter().any(|a| a == "--save");
    if canonical == "telegram" {
        return Err(cfgio::t(
            lang,
            "telegram has no detect: message your bot, then set chatId via 'channel set telegram --chat-id <id>'",
            "telegram 无需识别：向你的机器人发条消息后，用 'channel set telegram --chat-id <id>' 设置 chatId",
        ));
    }

    let cfg = AppConfig::load();
    // 第一步：校验凭据并取识别码。
    let code = cfgio::block_on(async {
        match canonical {
            "dingding" => {
                crate::commands::dingtalk_detect_prepare(crate::commands::DingTalkDetectArgs {
                    client_id: cfg.channels.dingding.client_id.clone(),
                    client_secret: String::new(),
                })
                .await
            }
            "feishu" => {
                crate::commands::feishu_detect_prepare(crate::commands::FeishuDetectArgs {
                    app_id: cfg.channels.feishu.app_id.clone(),
                    app_secret: String::new(),
                    base_url: cfg.channels.feishu.base_url.clone(),
                })
                .await
            }
            "slack" => {
                crate::commands::slack_detect_prepare(crate::commands::SlackDetectArgs {
                    bot_token: String::new(),
                    app_token: String::new(),
                })
                .await
            }
            _ => unreachable!(),
        }
    })?;

    eprintln!(
        "{}",
        cfgio::t(
            lang,
            &format!("Send this code to your bot within 120s: {code}"),
            &format!("请在 120 秒内把以下识别码发给你的机器人: {code}"),
        )
    );

    // 第二步：等待用户回发识别码，取得 id。
    let id = cfgio::block_on(async {
        match canonical {
            "dingding" => {
                crate::commands::dingtalk_detect_wait(crate::commands::DingTalkWaitArgs {
                    client_id: cfg.channels.dingding.client_id.clone(),
                    client_secret: String::new(),
                    code: code.clone(),
                })
                .await
            }
            "feishu" => {
                crate::commands::feishu_detect_wait(crate::commands::FeishuWaitArgs {
                    app_id: cfg.channels.feishu.app_id.clone(),
                    app_secret: String::new(),
                    base_url: cfg.channels.feishu.base_url.clone(),
                    code: code.clone(),
                })
                .await
            }
            "slack" => {
                crate::commands::slack_detect_wait(crate::commands::SlackWaitArgs {
                    bot_token: String::new(),
                    app_token: String::new(),
                    code: code.clone(),
                })
                .await
            }
            _ => unreachable!(),
        }
    })?;

    let field_label = match canonical {
        "feishu" => "openId",
        _ => "userId",
    };
    print_line(&id);

    let do_save = save
        || (cfgio::stdin_is_tty()
            && yes_no(
                &cfgio::t(
                    lang,
                    &format!("Save {field_label} = {id} to config?"),
                    &format!("把 {field_label} = {id} 保存到配置?"),
                ),
                true,
            )?);
    if do_save {
        let mut c = AppConfig::load_without_secrets();
        match canonical {
            "dingding" => c.channels.dingding.user_id = id.clone(),
            "feishu" => c.channels.feishu.open_id = id.clone(),
            "slack" => c.channels.slack.user_id = id.clone(),
            _ => {}
        }
        c.save().map_err(|e| e.to_string())?;
        eprintln!("{}", cfgio::t(lang, "saved", "已保存"));
    }
    Ok(())
}

// ——— 交互向导 ———

fn wizard(name: &str, lang: Lang) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    eprintln!(
        "{}",
        cfgio::t(
            lang,
            &format!("Configuring channel: {name} (press Enter to keep current value)"),
            &format!("配置渠道: {name}（回车保留当前值）"),
        )
    );

    match name {
        "telegram" => {
            let c = &mut cfg.channels.telegram;
            c.enabled = yes_no(
                &cfgio::t(lang, "Enable this channel?", "启用该渠道?"),
                c.enabled,
            )?;
            c.chat_id = cfgio::prompt_line("chatId", &c.chat_id)?;
            c.api_base_url = cfgio::prompt_line("apiBaseUrl", &c.api_base_url)?;
            prompt_secret_into(&mut c.bot_token, "botToken", lang)?;
        }
        "dingding" => {
            let c = &mut cfg.channels.dingding;
            c.enabled = yes_no(
                &cfgio::t(lang, "Enable this channel?", "启用该渠道?"),
                c.enabled,
            )?;
            c.client_id = cfgio::prompt_line("clientId", &c.client_id)?;
            c.user_id = cfgio::prompt_line("userId", &c.user_id)?;
            c.card_template_id = cfgio::prompt_line("cardTemplateId", &c.card_template_id)?;
            c.permission_confirm_card_template_id = cfgio::prompt_line(
                "permissionConfirmCardTemplateId",
                &c.permission_confirm_card_template_id,
            )?;
            prompt_secret_into(&mut c.client_secret, "clientSecret", lang)?;
        }
        "feishu" => {
            let c = &mut cfg.channels.feishu;
            c.enabled = yes_no(
                &cfgio::t(lang, "Enable this channel?", "启用该渠道?"),
                c.enabled,
            )?;
            c.app_id = cfgio::prompt_line("appId", &c.app_id)?;
            c.open_id = cfgio::prompt_line("openId", &c.open_id)?;
            c.base_url = cfgio::prompt_line("baseUrl", &c.base_url)?;
            prompt_secret_into(&mut c.app_secret, "appSecret", lang)?;
        }
        "slack" => {
            let c = &mut cfg.channels.slack;
            c.enabled = yes_no(
                &cfgio::t(lang, "Enable this channel?", "启用该渠道?"),
                c.enabled,
            )?;
            c.user_id = cfgio::prompt_line("userId", &c.user_id)?;
            prompt_secret_into(&mut c.bot_token, "botToken", lang)?;
            prompt_secret_into(&mut c.app_token, "appToken", lang)?;
        }
        _ => unreachable!(),
    }

    cfg.save().map_err(|e| e.to_string())?;
    print_line(&cfgio::t(
        lang,
        &format!("{name} saved"),
        &format!("{name} 已保存"),
    ));
    eprintln!(
        "{}",
        cfgio::t(
            lang,
            "Tip: run 'channel detect' to auto-fill the user id, and 'channel test' to verify.",
            "提示：可运行 'channel detect' 自动识别用户 id，'channel test' 验证连通。",
        )
    );
    Ok(())
}

/// 隐藏输入一个密钥；留空保留原值（向导用）。
fn prompt_secret_into(field: &mut String, label: &str, lang: Lang) -> Result<(), String> {
    let suffix = if field.is_empty() {
        cfgio::t(lang, " (unset)", "（未设）")
    } else {
        cfgio::t(lang, " (set; Enter to keep)", "（已设；回车保留）")
    };
    let v = cfgio::prompt_hidden(&format!("{label}{suffix}"))?;
    if !v.is_empty() {
        *field = v;
    }
    Ok(())
}

fn yes_no(label: &str, default_yes: bool) -> Result<bool, String> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    let ans = cfgio::prompt_line(&format!("{label} [{hint}]"), "")?;
    let ans = ans.trim().to_ascii_lowercase();
    if ans.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(ans.as_str(), "y" | "yes" | "1" | "true"))
}

// ——— 辅助 ———

fn canon(name: &str, lang: Lang) -> Result<&'static str, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "telegram" | "tg" => Ok("telegram"),
        "dingding" | "dingtalk" | "ding" => Ok("dingding"),
        "feishu" | "lark" => Ok("feishu"),
        "slack" => Ok("slack"),
        other => Err(cfgio::t(
            lang,
            &format!("unknown channel: {other} (expected telegram|dingding|feishu|slack)"),
            &format!("未知渠道: {other}（应为 telegram|dingding|feishu|slack）"),
        )),
    }
}

/// 配置名 → daemon `StatusInfo.im_connections` 里的连接名（钉钉连接名为 "dingtalk"）。
pub(crate) fn conn_name(name: &str) -> &'static str {
    match name {
        "dingding" => "dingtalk",
        "feishu" => "feishu",
        "telegram" => "telegram",
        "slack" => "slack",
        _ => "",
    }
}

pub(crate) fn is_enabled(cfg: &AppConfig, name: &str) -> bool {
    match name {
        "telegram" => cfg.channels.telegram.enabled,
        "dingding" => cfg.channels.dingding.enabled,
        "feishu" => cfg.channels.feishu.enabled,
        "slack" => cfg.channels.slack.enabled,
        _ => false,
    }
}

fn set_enabled(cfg: &mut AppConfig, name: &str, on: bool) {
    match name {
        "telegram" => cfg.channels.telegram.enabled = on,
        "dingding" => cfg.channels.dingding.enabled = on,
        "feishu" => cfg.channels.feishu.enabled = on,
        "slack" => cfg.channels.slack.enabled = on,
        _ => {}
    }
}

/// 配置是否齐全：必填非密钥字段非空 + 必填密钥已设。
pub(crate) fn is_configured(cfg: &AppConfig, name: &str) -> bool {
    use crate::secrets::*;
    match name {
        "telegram" => {
            !cfg.channels.telegram.chat_id.trim().is_empty()
                && cfgio::secret_is_set(ACCOUNT_TELEGRAM_TOKEN)
        }
        "dingding" => {
            !cfg.channels.dingding.client_id.trim().is_empty()
                && !cfg.channels.dingding.user_id.trim().is_empty()
                && cfgio::secret_is_set(ACCOUNT_DINGTALK_SECRET)
        }
        "feishu" => {
            !cfg.channels.feishu.app_id.trim().is_empty()
                && !cfg.channels.feishu.open_id.trim().is_empty()
                && cfgio::secret_is_set(ACCOUNT_FEISHU_SECRET)
        }
        "slack" => {
            !cfg.channels.slack.user_id.trim().is_empty()
                && cfgio::secret_is_set(ACCOUNT_SLACK_BOT_TOKEN)
                && cfgio::secret_is_set(ACCOUNT_SLACK_APP_TOKEN)
        }
        _ => false,
    }
}

fn help(lang: Lang) -> String {
    cfgio::t(
        lang,
        "AskHuman channel — configure IM channels (telegram | dingding | feishu | slack)\n\
\n\
  channel list [--json]              Show enabled / configured / connected per channel\n\
  channel set <name>                 Interactive wizard (in a terminal, no flags)\n\
  channel set <name> [flags]         Non-interactive (scriptable); secrets via env/file/stdin only\n\
  channel enable|disable <name>      Toggle a channel on/off\n\
  channel test <name>                Send a test message using the stored config\n\
  channel detect <name> [--save]     Auto-detect userId/openId (dingding|feishu|slack)\n\
\n\
  Common flags:   --enable | --disable\n\
  telegram:       --chat-id <id>  --api-base-url <url>  --bot-token-{env <VAR>|file <path>|stdin}\n\
  dingding:       --client-id <id>  --user-id <id>  --card-template-id <id>\n\
                  --inline-small-text <bool>  --convert-text-to-docx <bool>\n\
                  --client-secret-{env <VAR>|file <path>|stdin}\n\
  feishu:         --app-id <id>  --open-id <id>  --base-url <url>  --app-secret-{env|file|stdin}\n\
  slack:          --user-id <id>  --bot-token-{env|file|stdin}  --app-token-{env|file|stdin}\n\
\n\
  Example: AskHuman channel set telegram --enable --chat-id 123 --bot-token-env TG_TOKEN",
        "AskHuman channel —— 配置 IM 渠道（telegram | dingding | feishu | slack）\n\
\n\
  channel list [--json]              显示各渠道 已启用 / 配置齐全 / 已连接\n\
  channel set <渠道>                 交互向导（在终端、且不带选项时）\n\
  channel set <渠道> [选项]          非交互（可脚本）；密钥仅经 env/file/stdin 传入\n\
  channel enable|disable <渠道>      启用 / 禁用某渠道\n\
  channel test <渠道>                用已存配置发送一条测试消息\n\
  channel detect <渠道> [--save]     自动识别 userId/openId（dingding|feishu|slack）\n\
\n\
  通用选项:   --enable | --disable\n\
  telegram:   --chat-id <id>  --api-base-url <url>  --bot-token-{env <变量>|file <路径>|stdin}\n\
  dingding:   --client-id <id>  --user-id <id>  --card-template-id <id>\n\
              --inline-small-text <bool>  --convert-text-to-docx <bool>\n\
              --client-secret-{env <变量>|file <路径>|stdin}\n\
  feishu:     --app-id <id>  --open-id <id>  --base-url <url>  --app-secret-{env|file|stdin}\n\
  slack:      --user-id <id>  --bot-token-{env|file|stdin}  --app-token-{env|file|stdin}\n\
\n\
  示例: AskHuman channel set telegram --enable --chat-id 123 --bot-token-env TG_TOKEN",
    )
}

fn print_line(s: &str) {
    super::print_line(s);
}
