//! Final failures use ordinary Ask cards; their answers are native control operations, not
//! Stop-hook output. Keep presentation and decision parsing independent of the daemon.

use serde_json::Value;

use super::{result::TurnResult, AgentKind};
use crate::i18n::Lang;
use crate::ipc::TaskRequest;
use crate::models::{OptionItem, Question};

pub fn task(
    kind: AgentKind,
    session: &str,
    project: &str,
    result: &TurnResult,
    can_continue: bool,
    lang: Lang,
) -> TaskRequest {
    let reason = result
        .summary
        .as_deref()
        .or(result.reason.as_deref())
        .unwrap_or(match lang {
            Lang::Zh => "Agent 因错误停止了本轮。",
            Lang::En => "The Agent ended this turn with an error.",
        });
    let reason: String = reason.chars().take(2000).collect();
    let mut task = super::stop::build_task(kind, session, project, &[], 0, Some(&reason), lang);
    if let Some(at) = chrono::DateTime::from_timestamp_millis(result.at as i64) {
        task.message.text = format!(
            "{} · {}\n\n{}",
            kind.label(),
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z"),
            task.message.text
        );
    }
    task.stop_confirmation = false;
    task.native_request_id = Some(format!(
        "failure:{}:{}:{}",
        kind.as_str(),
        session,
        result.id
    ));
    task.caller_pid = 0;
    task.select_only = !can_continue;
    let (question, retry, acknowledge, unsupported) = match lang {
        Lang::Zh => (
            "Agent 本轮出错了，接下来怎么处理？",
            "要求继续",
            "知道了",
            "知道了（该 Agent 暂不支持接续）",
        ),
        Lang::En => (
            "This Agent turn failed. What should happen next?",
            "Request continuation",
            "Got it",
            "Got it (continuation is not supported for this Agent)",
        ),
    };
    task.questions = vec![Question::new(
        question.to_string(),
        if can_continue {
            vec![
                OptionItem::new(retry, true),
                OptionItem::new(acknowledge, false),
            ]
        } else {
            vec![OptionItem::new(unsupported, true)]
        },
    )];
    if !can_continue {
        task.message.text.push_str(match lang {
            Lang::Zh => "\n\n当前会话没有可用的原会话接续通道，请回到原 Agent 处理。",
            Lang::En => "\n\nThis session has no available continuation channel. Continue in the original Agent.",
        });
    }
    task
}

pub fn unavailable(task: &mut TaskRequest, reason: Option<&str>, lang: Lang) {
    let Some(detail) = control_reason(reason, lang) else {
        return;
    };
    task.questions[0].predefined_options[0].text = match lang {
        Lang::Zh => "知道了（暂时无法接续）",
        Lang::En => "Got it (continuation is temporarily unavailable)",
    }
    .into();
    task.message.text.push_str(&format!("\n\n{detail}"));
}

pub fn control_reason(reason: Option<&str>, lang: Lang) -> Option<&'static str> {
    Some(match (reason?, lang) {
        ("desktopDisconnected", Lang::Zh) => "原会话的桌面连接已断开，请恢复连接后在原会话继续。",
        ("desktopDisconnected", Lang::En) => "The desktop connection is disconnected. Reconnect and continue in the original chat.",
        ("controlStarting", Lang::Zh) => "已有一次接续正在确认，请先查看原会话，避免重复提交。",
        ("controlStarting", Lang::En) => "Another continuation is being confirmed. Check the original chat before submitting again.",
        ("stateChanged" | "resultChanged", Lang::Zh) => "原会话或回合状态已经变化，请先查看原会话。",
        ("stateChanged" | "resultChanged", Lang::En) => "The chat or turn has changed. Check the original chat first.",
        ("ended" | "notFound", Lang::Zh) => "原会话已经结束或无法找到，请回到原 Agent 检查。",
        ("ended" | "notFound", Lang::En) => "The original chat has ended or cannot be found. Check it in the Agent.",
        ("nativeUnavailable", Lang::Zh) => "原会话的控制通道暂不可用，请恢复连接后检查原会话。",
        ("nativeUnavailable", Lang::En) => "The chat's control channel is temporarily unavailable. Reconnect and check the original chat.",
        _ => return None,
    })
}

/// The acknowledgment always wins over an attached instruction. An unavailable continuation
/// never interprets free text as a successful send.
pub fn instruction(stdout: &str, can_continue: bool) -> Option<String> {
    if !can_continue {
        return None;
    }
    let value: Value = serde_json::from_str(stdout).ok()?;
    if value["action"] != "answer" {
        return None;
    }
    let answer = value["answers"].as_array()?.first()?;
    let indices = answer["selected_indices"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if indices.iter().any(|value| value.as_u64() == Some(1)) {
        return None;
    }
    if indices
        .iter()
        .any(|value| value.as_u64().is_none_or(|n| n > 1))
    {
        return None;
    }
    let text = answer["user_input"].as_str().unwrap_or_default().trim();
    let has_files = answer["files"]
        .as_array()
        .is_some_and(|files| files.iter().any(Value::is_string));
    if text.is_empty() && !has_files && !indices.iter().any(|value| value.as_u64() == Some(0)) {
        return None;
    }
    let text: String = text.chars().take(1000).collect();
    let files: Vec<String> = answer["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    Some(crate::integrations::agent_launch::task_with_attachments(
        &text,
        &files,
        &[],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn answer(indices: Value, text: &str) -> String {
        json!({"action":"answer","answers":[{"selected_indices":indices,"user_input":text}]})
            .to_string()
    }
    #[test]
    fn acknowledgment_cancel_and_unavailable_controls_never_send() {
        assert_eq!(
            instruction(&answer(json!([1]), "do not send this"), true),
            None
        );
        assert_eq!(
            instruction(&answer(json!([0, 1]), "do not send"), true),
            None
        );
        assert_eq!(instruction(&answer(json!([0]), "ignored"), false), None);
        assert_eq!(
            instruction(&answer(json!([5]), "invalid index"), true),
            None
        );
        assert_eq!(instruction("{\"action\":\"cancel\"}", true), None);
        assert_eq!(instruction(&answer(json!([]), ""), true), None);
    }
    #[test]
    fn continuation_and_free_text_preserve_bounded_user_input() {
        assert_eq!(
            instruction(&answer(json!([0]), ""), true),
            Some(String::new())
        );
        assert_eq!(
            instruction(&answer(json!([]), " continue here "), true),
            Some("continue here".into())
        );
        assert_eq!(
            instruction(&answer(json!([0]), &"字".repeat(1001)), true)
                .unwrap()
                .chars()
                .count(),
            1000
        );
    }
    #[test]
    fn a_failure_is_not_a_natural_stop_or_a_replayable_equal_question() {
        let result = TurnResult {
            id: "r1".into(),
            outcome: super::super::result::TurnOutcome::Failed,
            source: None,
            turn_id: None,
            reason: None,
            summary: Some("capacity".into()),
            at: 0,
        };
        let card = task(AgentKind::Codex, "s", "/project", &result, false, Lang::Zh);
        assert!(!card.stop_confirmation);
        assert!(card.record_history && card.select_only && card.single);
        assert_eq!(card.questions[0].predefined_options.len(), 1);
        assert!(card.questions[0].predefined_options[0]
            .text
            .contains("暂不支持接续"));
        let mut newer = result.clone();
        newer.id = "r2".into();
        assert_ne!(
            card.native_request_id,
            task(AgentKind::Codex, "s", "/project", &newer, false, Lang::Zh).native_request_id
        );
        let mut disconnected = card.clone();
        unavailable(&mut disconnected, Some("desktopDisconnected"), Lang::Zh);
        assert!(disconnected.questions[0].predefined_options[0]
            .text
            .contains("暂时无法接续"));
        assert!(disconnected.message.text.contains("桌面连接已断开"));
        assert_eq!(
            instruction(&answer(json!([0]), ""), !disconnected.select_only),
            None
        );
    }
}
