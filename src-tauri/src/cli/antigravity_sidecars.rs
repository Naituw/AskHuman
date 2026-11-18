//! `AskHuman agents sidecars [list|remove <id>|remove --all]`: the sidecars AskHuman keeps for the
//! Antigravity App (docs/specs/antigravity-deep-integration.md §12.3, §12.10). The daemon does the
//! work; this is the command line in front of it, for a machine without Settings.

use super::cfgio;
use crate::antigravity_desktop::Operation;
use crate::i18n::Lang;
use serde_json::Value;

pub fn run(args: &[String], lang: Lang) -> Result<(), String> {
    let json = args.iter().any(|arg| arg == "--json");
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--json")
        .collect();
    match rest.as_slice() {
        [] | ["list"] => {
            let answer = ask(Operation::Sidecars, lang)?;
            if json {
                super::print_line(
                    &serde_json::to_string_pretty(&answer["sidecars"]).unwrap_or_default(),
                );
            } else {
                super::print_line(&listing(&answer, lang));
            }
            Ok(())
        }
        ["remove", "--all"] => {
            let answer = ask(Operation::RemoveAll, lang)?;
            super::print_line(&removal(&answer, lang));
            failures(&answer, lang)
        }
        ["remove", id] if !id.starts_with('-') => {
            let answer = ask(
                Operation::Remove {
                    id: (*id).to_string(),
                },
                lang,
            )?;
            super::print_line(&removal(&answer, lang));
            Ok(())
        }
        _ => Err(usage(lang)),
    }
}

fn usage(lang: Lang) -> String {
    cfgio::t(
        lang,
        "usage: agents sidecars [list [--json] | remove <id> | remove --all]",
        "用法: agents sidecars [list [--json] | remove <id> | remove --all]",
    )
}

/// Sends the operation to the daemon. A refusal on purpose comes back as an error with its
/// message; the daemon is started if it is not running.
fn ask(operation: Operation, lang: Lang) -> Result<Value, String> {
    let answer = tokio::runtime::Runtime::new()
        .map_err(|error| error.to_string())?
        .block_on(crate::client::antigravity_desktop(operation))?;
    if answer["status"] == "refused" {
        return Err(refusal(&answer, lang));
    }
    Ok(answer)
}

fn refusal(answer: &Value, lang: Lang) -> String {
    let message = answer["message"].as_str().unwrap_or_default();
    match answer["reason"].as_str() {
        Some("notManaged") => cfgio::t(
            lang,
            &format!("AskHuman does not manage that sidecar ({message})"),
            &format!("AskHuman 没有管理这个 sidecar（{message}）"),
        ),
        Some("conflict") => cfgio::t(
            lang,
            &format!("Left alone, because it is not provably AskHuman's: {message}"),
            &format!("没有动它，因为无法证明它是 AskHuman 的：{message}"),
        ),
        _ => message.to_string(),
    }
}

fn failures(answer: &Value, lang: Lang) -> Result<(), String> {
    let failed = answer["failed"].as_array().map_or(0, Vec::len);
    if failed == 0 {
        return Ok(());
    }
    Err(cfgio::t(
        lang,
        &format!("{failed} sidecar(s) could not be removed"),
        &format!("有 {failed} 个 sidecar 没能移除"),
    ))
}

fn removal(answer: &Value, lang: Lang) -> String {
    let mut lines = Vec::new();
    for removed in answer["removed"].as_array().into_iter().flatten() {
        lines.push(cfgio::t(
            lang,
            &format!("removed {}", removed["id"].as_str().unwrap_or_default()),
            &format!("已移除 {}", removed["id"].as_str().unwrap_or_default()),
        ));
    }
    for failed in answer["failed"].as_array().into_iter().flatten() {
        let id = failed["id"].as_str().unwrap_or_default();
        let message = failed["message"].as_str().unwrap_or_default();
        lines.push(cfgio::t(
            lang,
            &format!("could not remove {id}: {message}"),
            &format!("无法移除 {id}：{message}"),
        ));
    }
    if lines.is_empty() {
        lines.push(cfgio::t(
            lang,
            "no sidecars to remove",
            "没有可移除的 sidecar",
        ));
    }
    lines.join("\n")
}

/// The list as text.
fn listing(answer: &Value, lang: Lang) -> String {
    let sidecars = answer["sidecars"].as_array().cloned().unwrap_or_default();
    let managing = answer["managing"] == true;
    let limit = answer["limit"].as_u64().unwrap_or(0);
    let mut lines = vec![cfgio::t(
        lang,
        &format!(
            "Antigravity sidecars: {} of {limit}; managing is {}",
            sidecars.len(),
            if managing { "on" } else { "off" }
        ),
        &format!(
            "Antigravity sidecar：{} / {limit}；管理开关{}",
            sidecars.len(),
            if managing { "已开" } else { "未开" }
        ),
    )];
    for sidecar in &sidecars {
        let kind = match sidecar["kind"].as_str() {
            Some("yolo") => cfgio::t(lang, "YOLO (grants everything)", "YOLO（宽授权）"),
            Some("control") => cfgio::t(lang, "messages only", "仅发消息"),
            _ => cfgio::t(lang, "whitelist", "白名单"),
        };
        let state = if sidecar["online"] == true {
            cfgio::t(lang, "online", "在线")
        } else {
            cfgio::t(lang, "offline", "离线")
        };
        lines.push(format!(
            "  {}  {kind}  {state}  {}",
            sidecar["id"].as_str().unwrap_or_default(),
            sidecar["directory"].as_str().unwrap_or_default()
        ));
        let mut notes = Vec::new();
        if sidecar["directoryExists"] == false {
            notes.push(cfgio::t(lang, "folder is gone", "目录已不存在"));
        }
        if sidecar["files"] == "missing" {
            notes.push(cfgio::t(
                lang,
                "files removed (not running until it is made again)",
                "文件已被删除（重新创建前不会运行）",
            ));
        }
        if let Some(conflict) = sidecar["conflict"].as_str() {
            notes.push(cfgio::t(
                lang,
                &format!("not AskHuman's, left alone: {conflict}"),
                &format!("不是 AskHuman 的，已保持原样：{conflict}"),
            ));
        }
        if sidecar["tracked"] == false {
            notes.push(cfgio::t(
                lang,
                "marked by AskHuman but not in its record; shown only",
                "带有 AskHuman 的标记，但不在它的记录里；仅显示",
            ));
        }
        if sidecar["enabled"] == false {
            notes.push(cfgio::t(
                lang,
                "switched off in Antigravity",
                "在 Antigravity 里被关闭",
            ));
        }
        let rules = sidecar["rules"].as_array().map_or(0, Vec::len);
        if rules > 0 {
            notes.push(cfgio::t(
                lang,
                &format!("{rules} rule(s) of its own"),
                &format!("{rules} 条自有规则"),
            ));
        }
        if let Some(used) = sidecar["lastUsedAt"].as_u64().filter(|used| *used > 0) {
            if let Some(time) = chrono::DateTime::from_timestamp_millis(used as i64) {
                notes.push(cfgio::t(
                    lang,
                    &format!(
                        "last used {}",
                        time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
                    ),
                    &format!(
                        "最近使用 {}",
                        time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
                    ),
                ));
            }
        }
        if !notes.is_empty() {
            lines.push(format!("      {}", notes.join(" · ")));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "status": "done",
            "managing": true,
            "limit": 20,
            "sidecars": [
                {"id": "askhuman-ws-1a2b3c4d", "kind": "workspace", "directory": "/Users/me/project",
                 "directoryExists": true, "rules": ["command(ls)", "read_url(example.com)"],
                 "createdAt": 1, "lastUsedAt": 0, "files": "present", "enabled": true,
                 "projectId": "p-1", "tracked": true, "online": true},
                {"id": "askhuman-yolo-aaaaaaaa", "kind": "yolo", "directory": "/gone",
                 "directoryExists": false, "rules": [], "createdAt": 1, "lastUsedAt": 0,
                 "files": "conflict", "conflict": "the manifest was edited", "enabled": false,
                 "projectId": null, "tracked": true, "online": false},
            ]
        })
    }

    #[test]
    fn the_list_names_each_sidecar_its_kind_and_what_is_off() {
        let text = listing(&sample(), Lang::En);
        assert!(text.starts_with("Antigravity sidecars: 2 of 20; managing is on"));
        assert!(text.contains("askhuman-ws-1a2b3c4d  whitelist  online  /Users/me/project"));
        assert!(text.contains("2 rule(s) of its own"));
        assert!(text.contains("askhuman-yolo-aaaaaaaa  YOLO (grants everything)  offline  /gone"));
        assert!(text.contains("folder is gone"));
        assert!(text.contains("not AskHuman's, left alone: the manifest was edited"));
        assert!(text.contains("switched off in Antigravity"));
        let zh = listing(&sample(), Lang::Zh);
        assert!(zh.contains("管理开关已开"));
        assert!(zh.contains("YOLO（宽授权）"));
        assert!(zh.contains("目录已不存在"));
    }

    #[test]
    fn an_empty_list_is_just_the_header() {
        let text = listing(
            &json!({"status": "done", "managing": false, "limit": 20, "sidecars": []}),
            Lang::En,
        );
        assert_eq!(text, "Antigravity sidecars: 0 of 20; managing is off");
    }

    #[test]
    fn a_removal_says_what_went_and_what_did_not() {
        let answer = json!({
            "status": "done",
            "removed": [{"id": "askhuman-ws-1"}],
            "failed": [{"id": "askhuman-ws-2", "message": "edited"}],
        });
        let text = removal(&answer, Lang::En);
        assert!(text.contains("removed askhuman-ws-1"));
        assert!(text.contains("could not remove askhuman-ws-2: edited"));
        assert!(failures(&answer, Lang::En).is_err());
        assert_eq!(
            removal(
                &json!({"status": "done", "removed": [], "failed": []}),
                Lang::En
            ),
            "no sidecars to remove"
        );
        assert!(failures(&json!({"removed": [], "failed": []}), Lang::En).is_ok());
    }

    #[test]
    fn a_refusal_is_explained() {
        let refused = json!({"status": "refused", "reason": "conflict", "message": "edited"});
        assert!(refusal(&refused, Lang::En).contains("not provably AskHuman's: edited"));
        let refused = json!({"status": "refused", "reason": "notManaged", "message": "x"});
        assert!(refusal(&refused, Lang::Zh).contains("没有管理"));
    }

    #[test]
    fn unknown_arguments_print_the_usage() {
        for args in [
            vec!["remove"],
            vec!["remove", "--force"],
            vec!["delete", "x"],
            vec!["list", "extra"],
        ] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            assert!(run(&args, Lang::En).unwrap_err().starts_with("usage:"));
        }
    }
}
