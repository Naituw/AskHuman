//! Cursor's opt-in away receipt releases the caller while a durable read-only card remains.

use super::*;
use crate::daemon::away::{AutoKind, AutoReply};
use crate::daemon::recovery::{Event, Outcome, OwnedEffect, RequestContext, Task};
use crate::models::{EscalationReason, OptionItem, Question};
use anyhow::{Context, Result};

const PREFIX: &str = "cursor-away-report/";
pub(crate) const RECEIPT: &str = "[cursor_away_report_saved]";

pub(super) fn is_report(id: Option<&str>) -> bool {
    id.is_some_and(|id| id.starts_with(PREFIX))
}

fn eligible(task: &TaskRequest, enabled: bool, escalated: Option<EscalationReason>) -> bool {
    enabled
        && task.agent_kind.as_deref().and_then(AgentKind::parse) == Some(AgentKind::Cursor)
        && escalated == Some(EscalationReason::Asked)
        && crate::daemon::away::is_agent_ask(task)
        && task
            .escalation_reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty())
}

pub(super) fn saved_receipt(
    task: &TaskRequest,
    context: Option<&RequestContext>,
) -> Result<Option<AutoReply>> {
    let Some(context) = context else {
        return Ok(None);
    };
    if !context
        .store
        .owned_exists(&format!("{PREFIX}{}", context.id))?
    {
        return Ok(None);
    }
    Ok(Some(AutoReply {
        kind: AutoKind::Away,
        text: receipt(Lang::resolve(&task.lang), &context.id),
    }))
}

pub(super) async fn intercept(
    state: &Arc<ServerState>,
    task: &TaskRequest,
    escalated: Option<EscalationReason>,
    context: Option<&RequestContext>,
) -> Result<Option<AutoReply>> {
    if !eligible(
        task,
        state
            .config_snapshot()
            .cursor_desktop
            .end_escalations_while_away,
        escalated,
    ) {
        return Ok(None);
    }
    let lang = Lang::resolve(&task.lang);
    let project = task.project.clone();
    let whats_next = task.whats_next;
    let automatic_work = tokio::task::spawn_blocking(move || {
        if !whats_next {
            crate::todo_sync::service::before_whats_next(&project);
        }
        crate::todos::first_auto(&project).is_some()
    })
    .await
    .context("cannot check automatic todos")?;
    if automatic_work {
        return Ok(Some(AutoReply {
            kind: AutoKind::Away,
            text: match lang {
                Lang::Zh => "项目里还有 ⚡ 自动待办。请调用 AskHuman --whats-next（MCP whats_next）领取并执行，现有队列用尽后再自查并带理由升级汇报。".into(),
                Lang::En => "Automatic project todos remain. Call AskHuman --whats-next (MCP whats_next) to receive and execute them. Once the queue is exhausted, reassess and escalate with a reason to report.".into(),
            },
        }));
    }
    let runtime = state
        .recovery
        .as_ref()
        .context("recovery storage is unavailable")?;
    let id = context
        .map(|context| context.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let key = format!("{PREFIX}{id}");
    // Commit the unread source before acknowledging the original caller. The maintenance loop
    // submits it independently, including after a crash between this commit and presentation.
    runtime
        .store
        .owned(&key, &Task::Ask(Box::new(report_task(task, &key))))?;
    Ok(Some(AutoReply {
        kind: AutoKind::Away,
        text: receipt(lang, &id),
    }))
}

fn receipt(lang: Lang, id: &str) -> String {
    let text = match lang {
        Lang::Zh => "用户已开启 Cursor 离开汇报后结束。汇报已持久保存为待阅汇报，用户回来后会阅读并点已阅。现在请结束本轮，不再调用 AskHuman 或等待已阅，也不要自动重试或发送后续消息。此回执代表用户对此情形的预先授权，可直接结束，无需再次调用 whats_next。最终输出末行附上",
        Lang::En => "The user enabled ending Cursor turns after away reports. Your report is durably saved for the user to read and acknowledge later. End this turn now. Do not call AskHuman again, wait for acknowledgement, retry automatically, or send follow-up messages. This receipt carries the user's prior authorization to end in this case, including from ask, without another whats_next call. Append this marker on the final line of your final output",
    };
    format!(
        "{RECEIPT}\n{id}\n\n{text}\n{}",
        crate::prompts::USER_CONFIRMED_END_TURN_MARKER
    )
}

fn report_task(original: &TaskRequest, key: &str) -> TaskRequest {
    let mut task = original.clone();
    let (title, hint, reason, question, read) = match Lang::resolve(&task.lang) {
        Lang::Zh => (
            "待阅汇报",
            "汇报已保留，可在方便时阅读并标记已阅。",
            "升级理由",
            "原提问",
            "已阅",
        ),
        Lang::En => (
            "Unread report",
            "This report is saved for you to read and acknowledge when convenient.",
            "Escalation reason",
            "Original question",
            "Read",
        ),
    };
    let reason_text = original.escalation_reason.as_deref().unwrap_or("").trim();
    let reason_text = if original.is_markdown {
        crate::msg_card::escape_markdown(reason_text)
    } else {
        reason_text.to_string()
    };
    let mut text = format!(
        "{title}\n\n{hint}\n\n{reason}\n{}\n\n{}",
        reason_text, original.message.text
    );
    for item in &original.questions {
        text.push_str(&format!("\n\n{question}\n{}", item.message));
        if !item.predefined_options.is_empty() {
            text.push('\n');
        }
        for option in &item.predefined_options {
            text.push_str(&format!("\n- {}", option.text));
        }
    }
    task.message.text = text;
    task.questions = vec![Question::new(
        title.into(),
        vec![OptionItem::new(read, false)],
    )];
    task.source = format!("{} · {title}", original.source);
    task.native_request_id = Some(key.into());
    task.select_only = true;
    task.single = true;
    task.whats_next = false;
    task.stop_confirmation = false;
    task.escalate = false;
    task.escalation_reason = None;
    task.caller_pid = 0;
    task.agent_pid = None;
    task.perf_id.clear();
    task.perf_autodismiss = false;
    task
}

/// Owned metadata is the unread source. Polling reuses the same call and never starts a daemon
/// through a client reconnect. Explicit shutdown leaves it waiting for the next ordinary start.
pub(super) async fn maintain(state: &Arc<ServerState>) -> Result<()> {
    let Some(runtime) = &state.recovery else {
        return Ok(());
    };
    if runtime.is_restoring()
        || runtime.suspending.load(Ordering::SeqCst)
        || state.closing.load(Ordering::SeqCst)
    {
        return Ok(());
    }
    for (key, record) in runtime.store.owned_records(PREFIX)? {
        if state.closing.load(Ordering::SeqCst) || runtime.suspending.load(Ordering::SeqCst) {
            break;
        }
        if record.effect != OwnedEffect::Waiting {
            continue;
        }
        let Some(task) = record.task else { continue };
        let record = runtime.store.owned(&key, &task)?;
        let events = runtime.submit(&record.call, task, state).await?;
        let event = events.borrow().clone();
        if let Some(Event::Final { outcome }) = event {
            let decided = runtime
                .store
                .resume(&record.call, None)?
                .is_some_and(|call| call.decided);
            if decided {
                runtime.store.complete_owned(&key, true)?;
            } else if !matches!(outcome, Outcome::Cancelled) {
                // A delivery failure is not an acknowledgement. Renew the owned call next tick.
                runtime.store.retry_owned(&key)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task() -> TaskRequest {
        serde_json::from_value(json!({
            "message":{"text":"Done", "files":[{"path":"/report.md","name":"report.md","size":4,"isImage":false}]},
            "questions":[{"message":"Deploy?", "predefinedOptions":["Deploy"]}],
            "isMarkdown":true,"source":"Cursor","lang":"en","project":"/project",
            "agentKind":"cursor","agentSessionId":"session","callerPid":123,
            "escalationReason":"The objective is complete"
        })).unwrap()
    }

    #[test]
    fn only_opted_in_cursor_agent_escalations_qualify() {
        let mut task = task();
        assert!(eligible(&task, true, Some(EscalationReason::Asked)));
        task.whats_next = true;
        assert!(eligible(&task, true, Some(EscalationReason::Asked)));
        assert!(!eligible(&task, false, Some(EscalationReason::Asked)));
        for reason in [
            None,
            Some(EscalationReason::Repeated),
            Some(EscalationReason::Requested),
        ] {
            assert!(!eligible(&task, true, reason));
        }
        task.agent_kind = Some("codex".into());
        assert!(!eligible(&task, true, Some(EscalationReason::Asked)));
        task.agent_kind = Some("cursor".into());
        for reason in [None, Some(" ".into())] {
            task.escalation_reason = reason;
            assert!(!eligible(&task, true, Some(EscalationReason::Asked)));
        }
    }

    #[test]
    fn report_keeps_readable_content_and_replaces_actions_with_acknowledgement() {
        let original = task();
        let report = report_task(&original, "cursor-away-report/one");
        assert!(report.message.text.contains("Done"));
        assert!(report.message.text.contains("The objective is complete"));
        assert!(report.message.text.contains("Deploy?\n\n- Deploy"));
        assert_eq!(report.message.files[0].path, "/report.md");
        assert_eq!(report.questions.len(), 1);
        assert_eq!(report.questions[0].predefined_options[0].text, "Read");
        assert!(report.select_only && report.single);
        assert_eq!(report.popup_kind(), ipc::PopupRequestKind::Ask);
        assert!(!crate::daemon::away::is_agent_ask(&report));
        assert!(!marks_session_working(
            true,
            false,
            report.native_request_id.as_deref()
        ));
        assert_eq!(report.caller_pid, 0);
    }

    #[test]
    fn end_receipt_authorizes_both_tool_entries() {
        for lang in [Lang::Zh, Lang::En] {
            let text = receipt(lang, "one");
            assert!(text.contains(RECEIPT));
            assert!(text.ends_with(crate::prompts::USER_CONFIRMED_END_TURN_MARKER));
            assert!(text.contains("whats_next"));
        }
        let old: crate::cursor_desktop::Config = serde_json::from_value(json!({})).unwrap();
        assert!(!old.end_escalations_while_away);
        assert!(crate::prompts::cli_reference().contains(RECEIPT));
        assert!(crate::prompts::mcp_reference().contains(RECEIPT));
    }

    #[test]
    fn a_saved_report_survives_stop_and_keeps_the_original_receipt_until_read() {
        use crate::daemon::recovery::{Request, Store};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("requests.sqlite3");
        let original = task();
        let key = format!("{PREFIX}original");
        let report = Task::Ask(Box::new(report_task(&original, &key)));
        let first = {
            let store = Store::open(&path).unwrap();
            let first = store.owned(&key, &report).unwrap();
            let request = Request {
                id: first.call.id.clone(),
                task: report.clone(),
                show: None,
                created_at_ms: 1,
            };
            store.accept(&request, &first.call, "hash").unwrap();
            store.finish(&request.id, &Outcome::Cancelled).unwrap();
            first
        };
        let store = Arc::new(Store::open(&path).unwrap());
        let context = RequestContext::new(store.clone(), "original".into(), true);
        let reply = saved_receipt(&original, Some(&context)).unwrap().unwrap();
        assert!(reply.text.contains("original"));
        let restored = store.owned(&key, &report).unwrap();
        assert_ne!(restored.call.id, first.call.id);
        assert_eq!(
            serde_json::to_value(&restored.task).unwrap(),
            serde_json::to_value(&first.task).unwrap()
        );
        let request = Request {
            id: restored.call.id.clone(),
            task: report.clone(),
            show: None,
            created_at_ms: 2,
        };
        store.accept(&request, &restored.call, "hash").unwrap();
        store
            .finish(
                &request.id,
                &Outcome::Ask(RenderOutcome {
                    stdout: String::new(),
                    stderr: Some("delivery failed".into()),
                    exit_code: 1,
                }),
            )
            .unwrap();
        store.retry_owned(&key).unwrap();
        let retry = store.owned(&key, &report).unwrap();
        assert_ne!(retry.call.id, restored.call.id);
        assert!(retry.effect == OwnedEffect::Waiting);
        assert!(retry.task.is_some());
        let request = Request {
            id: retry.call.id.clone(),
            task: report.clone(),
            show: None,
            created_at_ms: 3,
        };
        store.accept(&request, &retry.call, "hash").unwrap();
        store
            .decide(&request.id, &json!({"answer":{"action":"send"}}))
            .unwrap();
        store
            .finish(
                &request.id,
                &Outcome::Ask(RenderOutcome {
                    stdout: "Read".into(),
                    stderr: None,
                    exit_code: 0,
                }),
            )
            .unwrap();
        store.complete_owned(&key, true).unwrap();
        store.retry_owned(&key).unwrap();
        let read = store.owned(&key, &report).unwrap();
        assert!(read.effect == OwnedEffect::Complete);
        assert!(read.task.is_none());
        assert!(saved_receipt(&original, Some(&context)).unwrap().is_some());
    }
}
