//! `AskHuman channel ios <status|devices|pair|allow|deny|revoke|rename|export|import>` — management
//! of the iOS channel for headless use. The channel is experimental: these commands exist only
//! while the hidden `experimental.enabled` switch is on. Pairing state lives in the daemon's
//! gateway; the commands talk to it through `ClientMsg::Ios`.

use super::cfgio;
use crate::config::AppConfig;
use crate::i18n::Lang;
use crate::ios::gateway::{GatewayState, PhoneView};
use crate::ipc::{IosOp, IosResult};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Whether the experimental switch is on (the iOS channel is invisible otherwise).
pub fn available() -> bool {
    AppConfig::load_without_secrets().experimental.enabled
}

pub fn dispatch(args: &[String], lang: Lang) -> Result<(), String> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    match sub {
        "status" => status(rest, lang),
        "devices" | "ls" => devices(rest, lang),
        "pair" => pair(lang),
        "allow" => decide(true, lang),
        "deny" => decide(false, lang),
        "revoke" => revoke(rest, lang),
        "rename" => rename(rest, lang),
        "export" => export(lang),
        "import" => import(rest, lang),
        "help" | "-h" | "--help" => {
            print_line(&help(lang));
            Ok(())
        }
        other => Err(cfgio::t(
            lang,
            &format!("unknown subcommand: {other}\n\n{}", help(lang)),
            &format!("未知子命令: {other}\n\n{}", help(lang)),
        )),
    }
}

/// Gateway state from the daemon; `None` while the channel is disabled or not fully configured.
pub(crate) fn fetch_state() -> Result<Option<GatewayState>, String> {
    match cfgio::block_on(crate::client::ios_op(IosOp::State))? {
        IosResult::State { state } => Ok(state.map(|state| *state)),
        _ => Err("unexpected reply from the daemon".to_string()),
    }
}

fn running_state(lang: Lang) -> Result<GatewayState, String> {
    fetch_state()?.ok_or_else(|| {
        cfgio::t(
            lang,
            "the iOS channel is not running: enable it and complete the broker settings first ('channel set ios', 'channel enable ios')",
            "iOS 渠道未运行：请先填好 Broker 设置并启用（'channel set ios'、'channel enable ios'）",
        )
    })
}

// ——— status / devices ———

fn status(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let state = fetch_state()?;
    if json {
        let value = match &state {
            Some(s) => {
                let mut value = serde_json::to_value(s).unwrap_or_default();
                // The pairing code carries the broker password: never part of a status dump.
                if let Some(code) = value.pointer_mut("/pairing/code") {
                    *code = serde_json::Value::String("<redacted>".into());
                }
                value
            }
            None => serde_json::Value::Null,
        };
        print_line(&serde_json::to_string_pretty(&value).unwrap_or_default());
        return Ok(());
    }
    let Some(state) = state else {
        print_line(&cfgio::t(
            lang,
            "iOS channel: not running (disabled or broker settings incomplete)",
            "iOS 渠道：未运行（未启用或 Broker 设置不完整）",
        ));
        return Ok(());
    };
    let online = state.phones.iter().filter(|p| p.online).count();
    print_line(&format!(
        "{}: {} ({})",
        cfgio::t(lang, "Mac", "本机"),
        state.mac_name,
        state.mac_id
    ));
    print_line(&format!(
        "{}: {}",
        cfgio::t(lang, "Broker", "Broker"),
        if state.connected {
            cfgio::t(
                lang,
                &format!("connected via {}", state.via.as_deref().unwrap_or("?")),
                &format!("已连接（{}）", state.via.as_deref().unwrap_or("?")),
            )
        } else {
            cfgio::t(
                lang,
                &format!(
                    "disconnected{}",
                    state
                        .last_error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                ),
                &format!(
                    "未连接{}",
                    state
                        .last_error
                        .as_deref()
                        .map(|e| format!("（{e}）"))
                        .unwrap_or_default()
                ),
            )
        }
    ));
    print_line(&cfgio::t(
        lang,
        &format!(
            "Devices: {} ({online} online), pending requests: {}",
            state.phones.len(),
            state.pending_requests
        ),
        &format!(
            "设备：{} 台（{online} 台在线），待回答请求：{}",
            state.phones.len(),
            state.pending_requests
        ),
    ));
    if let Some(pairing) = &state.pairing {
        print_line(&cfgio::t(
            lang,
            &format!("Pairing in progress: {}", pairing.stage),
            &format!("配对进行中：{}", pairing.stage),
        ));
    }
    Ok(())
}

fn devices(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let state = running_state(lang)?;
    if json {
        print_line(&serde_json::to_string_pretty(&state.phones).unwrap_or_default());
        return Ok(());
    }
    if state.phones.is_empty() {
        print_line(&cfgio::t(
            lang,
            "No paired devices. Run 'channel ios pair' to pair one.",
            "还没有配对的设备。运行 'channel ios pair' 配对。",
        ));
        return Ok(());
    }
    print_line(&cfgio::t(
        lang,
        "id        name                     state     online  last seen",
        "id        名称                     状态      在线    最近在线",
    ));
    for phone in &state.phones {
        print_line(&format!(
            "{:<9} {:<24} {:<9} {:<7} {}",
            short_id(&phone.phone_id),
            display_name(phone),
            phone.state,
            if phone.online { "yes" } else { "no" },
            format_time(phone.last_seen_at)
        ));
    }
    Ok(())
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn display_name(phone: &PhoneView) -> String {
    phone
        .alias_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| phone.name.clone())
}

fn format_time(ms: i64) -> String {
    use chrono::TimeZone;
    if ms <= 0 {
        return "-".to_string();
    }
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Finds a device by full id, id prefix or name (alias first).
fn resolve_phone(state: &GatewayState, query: &str, lang: Lang) -> Result<PhoneView, String> {
    let query = query.trim();
    let by_id: Vec<&PhoneView> = state
        .phones
        .iter()
        .filter(|p| p.phone_id == query || (query.len() >= 4 && p.phone_id.starts_with(query)))
        .collect();
    let candidates: Vec<&PhoneView> = if by_id.is_empty() {
        state
            .phones
            .iter()
            .filter(|p| {
                display_name(p).eq_ignore_ascii_case(query) || p.name.eq_ignore_ascii_case(query)
            })
            .collect()
    } else {
        by_id
    };
    match candidates.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(cfgio::t(
            lang,
            &format!("no such device: {query} (see 'channel ios devices')"),
            &format!("没有这台设备：{query}（见 'channel ios devices'）"),
        )),
        _ => Err(cfgio::t(
            lang,
            &format!("more than one device matches: {query}; use its id"),
            &format!("多台设备匹配：{query}；请改用 id"),
        )),
    }
}

fn revoke(args: &[String], lang: Lang) -> Result<(), String> {
    let query = args.first().ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: channel ios revoke <device>",
            "用法: channel ios revoke <设备>",
        )
    })?;
    let state = running_state(lang)?;
    let phone = resolve_phone(&state, query, lang)?;
    cfgio::block_on(crate::client::ios_op(IosOp::Revoke {
        phone_id: phone.phone_id.clone(),
    }))?;
    print_line(&cfgio::t(
        lang,
        &format!("revoked {}", display_name(&phone)),
        &format!("已撤销 {}", display_name(&phone)),
    ));
    Ok(())
}

fn rename(args: &[String], lang: Lang) -> Result<(), String> {
    let (Some(query), Some(name)) = (args.first(), args.get(1)) else {
        return Err(cfgio::t(
            lang,
            "usage: channel ios rename <device> <new name>",
            "用法: channel ios rename <设备> <新名称>",
        ));
    };
    let state = running_state(lang)?;
    let phone = resolve_phone(&state, query, lang)?;
    cfgio::block_on(crate::client::ios_op(IosOp::Rename {
        phone_id: phone.phone_id.clone(),
        name: name.clone(),
    }))?;
    print_line(&cfgio::t(lang, "renamed", "已重命名"));
    Ok(())
}

// ——— pairing ———

fn start_pairing(lang: Lang) -> Result<(String, String, i64), String> {
    running_state(lang)?;
    match cfgio::block_on(crate::client::ios_op(IosOp::PairStart))? {
        IosResult::Pairing {
            ticket_id,
            code,
            expires_at_ms,
        } => Ok((ticket_id, code, expires_at_ms)),
        _ => Err("unexpected reply from the daemon".to_string()),
    }
}

fn pair(lang: Lang) -> Result<(), String> {
    let known: HashSet<String> = running_state(lang)?
        .phones
        .iter()
        .map(|p| p.phone_id.clone())
        .collect();
    let (ticket_id, code, expires_at_ms) = start_pairing(lang)?;
    let remaining = ((expires_at_ms - chrono::Utc::now().timestamp_millis()) / 1000).max(0);
    if let Ok(qr) = crate::ios::qr::text(&code) {
        eprintln!("{qr}");
    }
    eprintln!(
        "{}",
        cfgio::t(
            lang,
            &format!("Scan the code with the AskHuman app (valid for {remaining} s), or paste this pairing code into the app:"),
            &format!("用 AskHuman App 扫描二维码（{remaining} 秒内有效），或把下面的配对码粘贴到 App 中："),
        )
    );
    // The code carries the broker password: stdout only, never to a log.
    print_line(&code);

    let interactive = cfgio::stdin_is_tty();
    let hard_deadline = Instant::now() + Duration::from_secs(remaining as u64 + 150);
    let mut asked = false;
    loop {
        std::thread::sleep(Duration::from_millis(800));
        let state = running_state(lang)?;
        match state.pairing.as_ref().filter(|p| p.ticket_id == ticket_id) {
            Some(pairing) if pairing.stage == "pending" && !asked => {
                asked = true;
                eprintln!(
                    "{}",
                    cfgio::t(
                        lang,
                        &format!(
                            "\n\"{}\" wants to pair. Confirm that the app shows this code: {}",
                            pairing.phone_name.as_deref().unwrap_or("iPhone"),
                            pairing.sas.as_deref().unwrap_or("?")
                        ),
                        &format!(
                            "\n「{}」请求配对。请确认 App 上显示的确认码一致：{}",
                            pairing.phone_name.as_deref().unwrap_or("iPhone"),
                            pairing.sas.as_deref().unwrap_or("?")
                        ),
                    )
                );
                if !interactive {
                    eprintln!(
                        "{}",
                        cfgio::t(
                            lang,
                            "Not a terminal: finish with 'channel ios allow' or 'channel ios deny'.",
                            "当前不是终端：请用 'channel ios allow' 或 'channel ios deny' 完成。",
                        )
                    );
                    return Ok(());
                }
                let answer = cfgio::prompt_line(
                    &cfgio::t(lang, "Allow this device? [y/N]", "允许这台设备？[y/N]"),
                    "",
                )?;
                let allow = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                cfgio::block_on(crate::client::ios_op(IosOp::PairDecision {
                    ticket_id: ticket_id.clone(),
                    allow,
                }))?;
                if !allow {
                    print_line(&cfgio::t(lang, "denied", "已拒绝"));
                    return Ok(());
                }
            }
            Some(pairing) if pairing.stage == "waiting" => {
                if chrono::Utc::now().timestamp_millis() > expires_at_ms {
                    return Err(cfgio::t(lang, "the pairing code expired", "配对码已过期"));
                }
            }
            Some(_) => {}
            None => {
                let added: Vec<&PhoneView> = state
                    .phones
                    .iter()
                    .filter(|p| !known.contains(&p.phone_id))
                    .collect();
                return match added.first() {
                    Some(phone) => {
                        print_line(&cfgio::t(
                            lang,
                            &format!("paired: {}", display_name(phone)),
                            &format!("配对成功：{}", display_name(phone)),
                        ));
                        Ok(())
                    }
                    None => Err(cfgio::t(
                        lang,
                        "pairing ended without a new device (expired, denied or cancelled)",
                        "配对结束，没有新增设备（已过期、被拒绝或被取消）",
                    )),
                };
            }
        }
        if Instant::now() > hard_deadline {
            return Err(cfgio::t(lang, "pairing timed out", "配对超时"));
        }
    }
}

fn decide(allow: bool, lang: Lang) -> Result<(), String> {
    let state = running_state(lang)?;
    let Some(pairing) = state.pairing.filter(|p| p.stage == "pending") else {
        return Err(cfgio::t(
            lang,
            "no pairing request is waiting for a decision",
            "没有等待确认的配对请求",
        ));
    };
    cfgio::block_on(crate::client::ios_op(IosOp::PairDecision {
        ticket_id: pairing.ticket_id,
        allow,
    }))?;
    print_line(&if allow {
        cfgio::t(lang, "allowed", "已允许")
    } else {
        cfgio::t(lang, "denied", "已拒绝")
    });
    Ok(())
}

// ——— connection strings ———

fn export(lang: Lang) -> Result<(), String> {
    let cfg = AppConfig::load();
    let ios = &cfg.channels.ios;
    if ios.host.trim().is_empty() || ios.username.trim().is_empty() {
        return Err(cfgio::t(
            lang,
            "broker host and username are not configured yet",
            "尚未配置 Broker 主机和用户名",
        ));
    }
    eprintln!(
        "{}",
        cfgio::t(
            lang,
            "The connection string contains the broker password: share it only with your own devices.",
            "连接串包含 Broker 密码：只在你自己的设备之间传递。",
        )
    );
    print_line(&crate::ios::connstr::Connection::from_config(ios).encode());
    Ok(())
}

fn import(args: &[String], lang: Lang) -> Result<(), String> {
    let text = match args.first().map(|s| s.as_str()) {
        Some("-") | None => {
            if args.is_empty() && cfgio::stdin_is_tty() {
                return Err(cfgio::t(
                    lang,
                    "usage: channel ios import <ahconn1.…|->  (use - to read it from stdin)",
                    "用法: channel ios import <ahconn1.…|->（用 - 从 stdin 读取）",
                ));
            }
            cfgio::read_secret(&cfgio::SecretSource::Stdin, lang)?
        }
        Some(text) => text.to_string(),
    };
    let mut cfg = AppConfig::load_without_secrets();
    apply_connection(&mut cfg, &text)?;
    cfg.save().map_err(|e| e.to_string())?;
    print_line(&cfgio::t(
        lang,
        "broker settings imported (channel not enabled yet: 'channel enable ios')",
        "Broker 设置已导入（渠道尚未启用：'channel enable ios'）",
    ));
    Ok(())
}

/// Copies the broker settings of a connection string into the config (the channel's enabled
/// flag and display name are left alone).
pub(crate) fn apply_connection(cfg: &mut AppConfig, text: &str) -> Result<(), String> {
    let connection = crate::ios::connstr::Connection::decode(text)?;
    let ios = &mut cfg.channels.ios;
    ios.host = connection.host;
    ios.port = connection.port;
    ios.ws_port = connection.ws_port;
    ios.username = connection.username;
    ios.password = connection.password;
    ios.relay_url = connection.relay_url;
    Ok(())
}

pub(crate) fn help(lang: Lang) -> String {
    cfgio::t(
        lang,
        "AskHuman channel ios — the iOS app channel (experimental)\n\
\n\
  channel ios status [--json]        Broker connection, devices and pending requests\n\
  channel ios devices [--json]       Paired devices\n\
  channel ios pair                   Show a pairing QR code / code and confirm the device here\n\
  channel ios allow | deny           Decide a waiting pairing request (when not in a terminal)\n\
  channel ios revoke <device>        Unpair a device (id, id prefix or name)\n\
  channel ios rename <device> <name> Rename a device\n\
  channel ios export                 Print the connection string (includes the broker password)\n\
  channel ios import <text|->        Take over the broker settings of another Mac\n\
\n\
  Broker settings: AskHuman channel set ios --host <h> --username <u> --relay-url <url>\n\
                   --password-{env <VAR>|file <path>|stdin}  (or --connection-{env|file|stdin})",
        "AskHuman channel ios —— iOS App 渠道（实验功能）\n\
\n\
  channel ios status [--json]        Broker 连接、设备与待回答请求\n\
  channel ios devices [--json]       已配对设备\n\
  channel ios pair                   显示配对二维码 / 配对码，并在此确认设备\n\
  channel ios allow | deny           处理等待中的配对请求（不在终端时）\n\
  channel ios revoke <设备>          解除某台设备的配对（id、id 前缀或名称）\n\
  channel ios rename <设备> <名称>   重命名设备\n\
  channel ios export                 打印连接串（包含 Broker 密码）\n\
  channel ios import <文本|->        沿用另一台 Mac 的 Broker 设置\n\
\n\
  Broker 设置: AskHuman channel set ios --host <主机> --username <用户名> --relay-url <地址>\n\
               --password-{env <变量>|file <路径>|stdin}（或 --connection-{env|file|stdin}）",
    )
}

fn print_line(s: &str) {
    super::print_line(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ios::store::Traffic;

    fn phone(id: &str, name: &str, alias: Option<&str>) -> PhoneView {
        PhoneView {
            phone_id: id.into(),
            name: name.into(),
            alias_name: alias.map(str::to_string),
            state: "active".into(),
            online: true,
            foreground: false,
            created_at: 0,
            last_seen_at: 0,
            push: None,
            key_backend: None,
        }
    }

    fn state(phones: Vec<PhoneView>) -> GatewayState {
        GatewayState {
            connected: true,
            via: None,
            last_error: None,
            mac_id: "mac".into(),
            mac_name: "Mac".into(),
            phones,
            pairing: None,
            traffic: Traffic::default(),
            pending_requests: 0,
        }
    }

    #[test]
    fn devices_resolve_by_id_prefix_or_name() {
        let state = state(vec![
            phone("aaaa1111bbbb", "iPhone 16", None),
            phone("aaaa2222cccc", "iPad", Some("Work iPad")),
        ]);
        let find = |q: &str| resolve_phone(&state, q, Lang::En);
        assert_eq!(find("aaaa1111").unwrap().name, "iPhone 16");
        assert_eq!(find("aaaa1111bbbb").unwrap().name, "iPhone 16");
        assert_eq!(find("work ipad").unwrap().name, "iPad");
        assert_eq!(find("IPAD").unwrap().name, "iPad");
        assert!(find("aaaa").unwrap_err().contains("more than one"));
        assert!(find("nobody").unwrap_err().contains("no such device"));
    }

    #[test]
    fn a_connection_string_keeps_the_local_switches() {
        let mut cfg = AppConfig::default();
        cfg.channels.ios.enabled = true;
        cfg.channels.ios.mac_name = "Home".into();
        let text = crate::ios::connstr::Connection {
            host: "broker.example.com".into(),
            port: 8883,
            ws_port: 8084,
            username: "abc123".into(),
            password: "pw".into(),
            relay_url: "https://relay.example.com".into(),
        }
        .encode();
        apply_connection(&mut cfg, &text).unwrap();
        let ios = &cfg.channels.ios;
        assert!(ios.enabled);
        assert_eq!(ios.mac_name, "Home");
        assert_eq!(ios.host, "broker.example.com");
        assert_eq!(ios.password, "pw");
        assert!(apply_connection(&mut cfg, "nonsense").is_err());
    }
}
