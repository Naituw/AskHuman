//! `AskHuman config <show|get|set|unset|path|help>` —— 通用键值兜底（点号 camelCase 键）。
//! 密钥键自动路由进钥匙串，其值只从 stdin/env/file 取、绝不进 argv（见设计 §4 / D5）。

use super::cfgio::{self, SecretSource};
use crate::config::AppConfig;
use crate::i18n::{err_prefix, Lang};
use crate::secrets;
use serde_json::Value;
use std::process::exit;

pub fn dispatch(args: &[String], lang: Lang) {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let r = match sub {
        "show" | "list" => show(rest, lang),
        "get" => get(rest, lang),
        "set" => set(rest, lang),
        "unset" => unset(rest, lang),
        "path" => {
            print_line(&crate::paths::config_file().display().to_string());
            Ok(())
        }
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

fn show(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|a| a == "--json");
    let v = cfgio::redacted_value();
    if json {
        print_line(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        let mut lines = Vec::new();
        flatten(&v, "", &mut lines);
        for (k, val) in lines {
            print_line(&format!("{k} = {val}"));
        }
        print_line("");
        print_line(&cfgio::t(
            lang,
            "(secrets shown as ●●● when set; set them via: channel secret / config set <key> --from-env|--from-file|--from-stdin)",
            "（密钥已设时显示 ●●●；设置方式：channel secret 或 config set <键> --from-env|--from-file|--from-stdin）",
        ));
    }
    Ok(())
}

fn get(args: &[String], lang: Lang) -> Result<(), String> {
    let key = args
        .first()
        .ok_or_else(|| cfgio::t(lang, "usage: config get <key>", "用法: config get <键>"))?;
    let v = cfgio::redacted_value();
    match cfgio::get_path(&v, key) {
        Some(val) => {
            print_line(&value_to_plain(val));
            Ok(())
        }
        None => Err(cfgio::t(
            lang,
            &format!("unknown config key: {key}"),
            &format!("未知配置键: {key}"),
        )),
    }
}

fn set(args: &[String], lang: Lang) -> Result<(), String> {
    let key = args
        .first()
        .ok_or_else(|| {
            cfgio::t(
                lang,
                "usage: config set <key> <value>",
                "用法: config set <键> <值>",
            )
        })?
        .clone();

    let mut cfg = AppConfig::load_without_secrets();

    if cfgio::is_secret_key(&key) {
        // 密钥：忽略任何位置参数值，仅从 env/file/stdin/隐藏输入取。
        let src = secret_source_from_flags(&args[1..], lang)?;
        let val = cfgio::read_secret(&src, lang)?;
        assign_secret_field(&mut cfg, &key, val);
        cfg.save().map_err(|e| e.to_string())?;
        print_line(&cfgio::t(
            lang,
            &format!("{key} updated (stored in keychain)"),
            &format!("{key} 已更新（已存入钥匙串）"),
        ));
        return Ok(());
    }

    // 非密钥：需位置值。
    let value_str = args.get(1).ok_or_else(|| {
        cfgio::t(
            lang,
            "usage: config set <key> <value>",
            "用法: config set <键> <值>",
        )
    })?;
    let mut v = serde_json::to_value(&cfg).unwrap_or(Value::Null);
    let existing = cfgio::get_path(&v, &key)
        .ok_or_else(|| {
            cfgio::t(
                lang,
                &format!("unknown config key: {key}"),
                &format!("未知配置键: {key}"),
            )
        })?
        .clone();
    let coerced = cfgio::coerce_to_type(&existing, value_str)?;
    cfgio::set_path(&mut v, &key, coerced)?;
    cfg = serde_json::from_value(v).map_err(|e| {
        cfgio::t(
            lang,
            &format!("invalid value for {key}: {e}"),
            &format!("{key} 的值非法: {e}"),
        )
    })?;
    cfg.save().map_err(|e| e.to_string())?;
    print_line(&cfgio::t(
        lang,
        &format!("{key} updated"),
        &format!("{key} 已更新"),
    ));
    Ok(())
}

fn unset(args: &[String], lang: Lang) -> Result<(), String> {
    let key = args
        .first()
        .ok_or_else(|| cfgio::t(lang, "usage: config unset <key>", "用法: config unset <键>"))?;
    if cfgio::is_secret_key(key) {
        let _ = secrets::delete(key);
        let mut cfg = AppConfig::load_without_secrets();
        assign_secret_field(&mut cfg, key, String::new());
        cfg.save().map_err(|e| e.to_string())?;
        print_line(&cfgio::t(
            lang,
            &format!("{key} cleared"),
            &format!("{key} 已清除"),
        ));
        return Ok(());
    }
    // 非密钥：重置为默认值。
    let defaults = serde_json::to_value(AppConfig::default()).unwrap_or(Value::Null);
    let def = cfgio::get_path(&defaults, key)
        .ok_or_else(|| {
            cfgio::t(
                lang,
                &format!("unknown config key: {key}"),
                &format!("未知配置键: {key}"),
            )
        })?
        .clone();
    let mut cfg = AppConfig::load_without_secrets();
    let mut v = serde_json::to_value(&cfg).unwrap_or(Value::Null);
    cfgio::set_path(&mut v, key, def)?;
    cfg = serde_json::from_value(v).map_err(|e| e.to_string())?;
    cfg.save().map_err(|e| e.to_string())?;
    print_line(&cfgio::t(
        lang,
        &format!("{key} reset to default"),
        &format!("{key} 已重置为默认值"),
    ));
    Ok(())
}

/// 从 `--from-env <VAR>` / `--from-file <path>` / `--from-stdin` 决定密钥来源；
/// 都没有时：终端 → 隐藏输入；非终端 → 报错（避免脚本阻塞）。
fn secret_source_from_flags(flags: &[String], lang: Lang) -> Result<SecretSource, String> {
    let mut i = 0;
    while i < flags.len() {
        match flags[i].as_str() {
            "--from-env" => {
                let var = flags.get(i + 1).ok_or_else(|| {
                    cfgio::t(
                        lang,
                        "--from-env needs a variable name",
                        "--from-env 需要变量名",
                    )
                })?;
                return Ok(SecretSource::Env(var.clone()));
            }
            "--from-file" => {
                let p = flags.get(i + 1).ok_or_else(|| {
                    cfgio::t(lang, "--from-file needs a path", "--from-file 需要路径")
                })?;
                return Ok(SecretSource::File(p.clone()));
            }
            "--from-stdin" => return Ok(SecretSource::Stdin),
            _ => i += 1,
        }
    }
    if cfgio::stdin_is_tty() {
        Ok(SecretSource::Prompt(cfgio::t(lang, "value", "值")))
    } else {
        Err(cfgio::t(
            lang,
            "secret value required: pass --from-env <VAR> / --from-file <path> / --from-stdin (never as a plain argument)",
            "需要密钥值：用 --from-env <变量> / --from-file <路径> / --from-stdin（不要作为明文参数传入）",
        ))
    }
}

fn assign_secret_field(cfg: &mut AppConfig, key: &str, val: String) {
    match key {
        k if k == secrets::ACCOUNT_DINGTALK_SECRET => cfg.channels.dingding.client_secret = val,
        k if k == secrets::ACCOUNT_FEISHU_SECRET => cfg.channels.feishu.app_secret = val,
        k if k == secrets::ACCOUNT_TELEGRAM_TOKEN => cfg.channels.telegram.bot_token = val,
        k if k == secrets::ACCOUNT_SLACK_BOT_TOKEN => cfg.channels.slack.bot_token = val,
        k if k == secrets::ACCOUNT_SLACK_APP_TOKEN => cfg.channels.slack.app_token = val,
        _ => {}
    }
}

/// 把嵌套 `Value` 扁平成 `a.b.c = v` 行（仅标量；对象递归）。
fn flatten(v: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(val, &key, out);
            }
        }
        other => out.push((prefix.to_string(), value_to_plain(other))),
    }
}

fn value_to_plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn help(lang: Lang) -> String {
    cfgio::t(
        lang,
        "AskHuman config — generic key/value over ~/.askhuman/config.json (fallback; prefer 'channel' for IM setup)\n\
\n\
  config show [--json]            Print effective config (secrets shown as ●●●)\n\
  config get <key>                Print one value (dotted camelCase key)\n\
  config set <key> <value>        Set a non-secret key (e.g. general.language zh)\n\
  config set <key> --from-env <VAR>|--from-file <path>|--from-stdin   Set a secret key (→ keychain)\n\
  config unset <key>              Reset a key to its default (secret → cleared)\n\
  config path                     Print the config file path\n\
\n\
  Keys: general.* / channels.<name>.* / channels.autoActivation / experimental.enabled\n\
  Secret keys: channels.dingding.clientSecret, channels.feishu.appSecret,\n\
               channels.telegram.botToken, channels.slack.botToken, channels.slack.appToken",
        "AskHuman config —— 对 ~/.askhuman/config.json 的通用键值（兜底；渠道配置优先用 'channel'）\n\
\n\
  config show [--json]            打印生效配置（密钥显示为 ●●●）\n\
  config get <键>                 打印某个值（点号小驼峰键）\n\
  config set <键> <值>            设置非密钥键（如 general.language zh）\n\
  config set <键> --from-env <变量>|--from-file <路径>|--from-stdin   设置密钥键（→ 钥匙串）\n\
  config unset <键>               重置为默认（密钥 → 清除）\n\
  config path                     打印配置文件路径\n\
\n\
  键：general.* / channels.<渠道>.* / channels.autoActivation / experimental.enabled\n\
  密钥键：channels.dingding.clientSecret、channels.feishu.appSecret、\n\
          channels.telegram.botToken、channels.slack.botToken、channels.slack.appToken",
    )
}

/// stdout 输出一行（BrokenPipe 静默；见 `cli::print_line`）。
fn print_line(s: &str) {
    super::print_line(s);
}
