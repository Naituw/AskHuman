//! After a launch that must keep Max off: look at the CLI's persisted Max toggle for a short while
//! and, if it is on, say so with an ordinary AskHuman card (docs/specs/cursor-max-control.md §4.5).
//!
//! The helper in the new terminal turns the toggle off right before the Agent starts, so a toggle
//! that is still on a few seconds later means that did not work, and one that comes back on means
//! someone (or the Agent) switched it on again. Either way the user is told once; the task itself
//! is never touched.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::agents::{cursor_max::cli_config, AgentKind};
use crate::i18n::Lang;
use crate::integrations::agent_launch::LaunchRecord;
use crate::ipc::TaskRequest;
use crate::models::{OptionItem, Question};

/// How long the config is watched after the terminal was opened.
pub const WINDOW: Duration = Duration::from_secs(45);
/// How often it is read.
pub const INTERVAL: Duration = Duration::from_secs(1);
/// How long a toggle that was already on at the start may take to be turned off by the helper.
pub const SETTLE: Duration = Duration::from_secs(15);

/// What the watch found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finding {
    /// Nothing wrong within the window.
    Quiet,
    /// The toggle was on when the terminal was opened and never went off.
    Still,
    /// The toggle was off (or went off) and was switched on again.
    Reenabled,
}

/// Polls `is_on` every `interval` for `window`. A toggle that is on at the start is given `settle`
/// to be turned off before that counts as a finding.
pub async fn watch(
    window: Duration,
    interval: Duration,
    settle: Duration,
    mut is_on: impl FnMut() -> bool,
) -> Finding {
    let started = tokio::time::Instant::now();
    let mut settled = !is_on();
    loop {
        tokio::time::sleep(interval).await;
        let on = is_on();
        if !settled {
            if !on {
                settled = true;
            } else if started.elapsed() >= settle {
                return Finding::Still;
            }
        } else if on {
            return Finding::Reenabled;
        }
        if started.elapsed() >= window {
            return Finding::Quiet;
        }
    }
}

/// What a finished launch needs for the watch and the card.
#[derive(Debug, Clone)]
pub struct Launch {
    pub id: String,
    pub project: String,
    pub config: PathBuf,
}

/// The watch a launch needs, if any: only a Cursor record that keeps Max off, with a config file.
pub fn launch_of(record: &LaunchRecord) -> Option<Launch> {
    (record.kind == AgentKind::Cursor && record.allow_max == Some(false)).then_some(())?;
    Some(Launch {
        id: record.id.clone(),
        project: crate::project::detect_from(std::path::Path::new(&record.cwd)),
        config: cli_config::config_path()?,
    })
}

/// Watches after the terminal was opened and asks the user to acknowledge if Max is on. Meant to
/// be spawned by the process that opened the terminal.
pub async fn after_launch(launch: Launch) {
    let path = launch.config.clone();
    let finding = watch(WINDOW, INTERVAL, SETTLE, move || {
        cli_config::read_state_at(&path).is_some_and(|state| state.persisted_max())
    })
    .await;
    announce(finding, card(&launch, finding, Lang::current())).await;
}

/// How long a chat created in Cursor Desktop is watched, and how long a Max flag that was already
/// on at the start may stay on. The chat was checked before it was submitted, so any finding means
/// Cursor changed it afterwards.
pub const DESKTOP_WINDOW: Duration = Duration::from_secs(20);
pub const DESKTOP_SETTLE: Duration = Duration::from_secs(4);

/// A chat created in Cursor Desktop's Agents window with Max kept off.
#[derive(Debug, Clone)]
pub struct DesktopLaunch {
    /// The chat's id, which is also the session id.
    pub id: String,
    pub project: String,
}

/// Whether Cursor's own record of the chat says Max is on. A chat that cannot be read yet is not.
fn chat_max_on(id: &str) -> bool {
    crate::agents::cursor_vscdb::session_metadata(id).and_then(|value| {
        value
            .pointer("/modelConfig/maxMode")
            .and_then(Value::as_bool)
    }) == Some(true)
}

/// The Desktop twin of [`after_launch`]: reads the chat's Max flag from Cursor's database for a
/// short while after the task was submitted.
pub async fn after_desktop(launch: DesktopLaunch) {
    let id = launch.id.clone();
    let finding = watch(DESKTOP_WINDOW, INTERVAL, DESKTOP_SETTLE, move || {
        chat_max_on(&id)
    })
    .await;
    announce(finding, desktop_card(&launch, finding, Lang::current())).await;
}

async fn announce(finding: Finding, task: TaskRequest) {
    if finding == Finding::Quiet {
        return;
    }
    crate::daemon::lifecycle::log_runtime_event(
        "cursor_max",
        match finding {
            Finding::Still => "max_still_on_after_launch",
            _ => "max_switched_on_after_launch",
        },
        None,
    );
    let accepted = std::sync::Mutex::new(None::<String>);
    let _ = tokio::time::timeout(
        Duration::from_secs(86400),
        crate::client::internal_ask(task, &accepted),
    )
    .await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Cli,
    Desktop,
}

/// The acknowledge-only card for a terminal launch.
pub fn card(launch: &Launch, finding: Finding, lang: Lang) -> TaskRequest {
    build_card(&launch.id, &launch.project, Surface::Cli, finding, lang)
}

/// The acknowledge-only card for a chat created in the Agents window.
pub fn desktop_card(launch: &DesktopLaunch, finding: Finding, lang: Lang) -> TaskRequest {
    build_card(&launch.id, &launch.project, Surface::Desktop, finding, lang)
}

fn build_card(
    id: &str,
    project: &str,
    surface: Surface,
    finding: Finding,
    lang: Lang,
) -> TaskRequest {
    let (what, advice, question, ack) = match (lang, surface, finding) {
        (Lang::Zh, Surface::Cli, Finding::Still) => (
            "AskHuman 本次启动要求关闭 Max 模式，但 Cursor CLI 的配置里 Max 仍然开着。",
            "Max 在旧的按次计费计划里按 API 费率扣额度。请在该会话里输入 `/max-mode` 关闭，或结束该会话。",
            "已检测到 Max 开启，处理完请确认。",
            "知道了",
        ),
        (Lang::Zh, Surface::Cli, _) => (
            "刚启动的 Cursor 任务里，Max 模式被重新打开了。",
            "Max 在旧的按次计费计划里按 API 费率扣额度。若不是你的操作，请在该会话里输入 `/max-mode` 关闭，或结束该会话。",
            "已检测到 Max 开启，处理完请确认。",
            "知道了",
        ),
        (Lang::Zh, Surface::Desktop, Finding::Still) => (
            "AskHuman 本次创建要求关闭 Max 模式，但刚创建的 Cursor 会话里 Max 仍然开着。",
            "Max 在旧的按次计费计划里按 API 费率扣额度。请在该会话的模型选择器里关闭 Max，或结束该会话。",
            "已检测到 Max 开启，处理完请确认。",
            "知道了",
        ),
        (Lang::Zh, Surface::Desktop, _) => (
            "刚创建的 Cursor 会话里，Max 模式被重新打开了。",
            "Max 在旧的按次计费计划里按 API 费率扣额度。若不是你的操作，请在该会话的模型选择器里关闭 Max，或结束该会话。",
            "已检测到 Max 开启，处理完请确认。",
            "知道了",
        ),
        (Lang::En, Surface::Cli, Finding::Still) => (
            "This launch was asked to keep Max mode off, but it is still on in the Cursor CLI's config.",
            "On the request-based plan Max is billed at API rates. Turn it off in that session with `/max-mode`, or end the session.",
            "Max is on. Acknowledge when you have dealt with it.",
            "Got it",
        ),
        (Lang::En, Surface::Cli, _) => (
            "Max mode was switched on again in the Cursor task that was just started.",
            "On the request-based plan Max is billed at API rates. If that was not you, turn it off in that session with `/max-mode`, or end the session.",
            "Max is on. Acknowledge when you have dealt with it.",
            "Got it",
        ),
        (Lang::En, Surface::Desktop, Finding::Still) => (
            "This task was asked to keep Max mode off, but the Cursor chat that was just created has it on.",
            "On the request-based plan Max is billed at API rates. Turn it off in that chat's model picker, or end the chat.",
            "Max is on. Acknowledge when you have dealt with it.",
            "Got it",
        ),
        (Lang::En, Surface::Desktop, _) => (
            "Max mode was switched on again in the Cursor chat that was just created.",
            "On the request-based plan Max is billed at API rates. If that was not you, turn it off in that chat's model picker, or end the chat.",
            "Max is on. Acknowledge when you have dealt with it.",
            "Got it",
        ),
    };
    let heading = match lang {
        Lang::Zh => "Cursor Max 模式提醒",
        Lang::En => "Cursor Max mode warning",
    };
    let message = format!("**{heading}**\n\n{what}\n\n{advice}");
    let mut task = crate::agents::stop::build_task(
        AgentKind::Cursor,
        "",
        project,
        &[],
        0,
        Some(&message),
        lang,
    );
    task.stop_confirmation = false;
    task.native_request_id = Some(format!("cursor-max:{id}"));
    task.caller_pid = 0;
    task.select_only = true;
    task.agent_session_id = None;
    task.questions = vec![Question::new(
        question.to_string(),
        vec![OptionItem::new(ack, true)],
    )];
    task
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const TICK: Duration = Duration::from_millis(2);

    /// A script of what the toggle reads on successive polls; the last value repeats.
    fn script(values: &[bool]) -> impl FnMut() -> bool {
        let values = values.to_vec();
        let at = Arc::new(AtomicUsize::new(0));
        move || {
            let index = at.fetch_add(1, Ordering::SeqCst).min(values.len() - 1);
            values[index]
        }
    }

    async fn run(window_ms: u64, settle_ms: u64, values: &[bool]) -> Finding {
        watch(
            Duration::from_millis(window_ms),
            TICK,
            Duration::from_millis(settle_ms),
            script(values),
        )
        .await
    }

    #[tokio::test]
    async fn a_toggle_that_stays_off_is_quiet_and_the_watch_ends_with_the_window() {
        assert_eq!(run(30, 10, &[false]).await, Finding::Quiet);
    }

    #[tokio::test]
    async fn a_toggle_switched_on_later_is_found() {
        assert_eq!(
            run(200, 10, &[false, false, false, true]).await,
            Finding::Reenabled
        );
    }

    #[tokio::test]
    async fn a_toggle_that_was_on_and_goes_off_is_the_helper_doing_its_job() {
        assert_eq!(run(30, 20, &[true, true, false]).await, Finding::Quiet);
        // ...and coming back after that is a finding.
        assert_eq!(
            run(200, 20, &[true, false, false, true]).await,
            Finding::Reenabled
        );
    }

    #[tokio::test]
    async fn a_toggle_that_never_goes_off_is_found_once_the_settle_time_is_up() {
        assert_eq!(run(500, 20, &[true]).await, Finding::Still);
    }

    #[test]
    fn the_desktop_card_points_at_the_model_picker_not_the_cli_command() {
        let launch = DesktopLaunch {
            id: "abc".into(),
            project: "/p/demo".into(),
        };
        for lang in [Lang::Zh, Lang::En] {
            for finding in [Finding::Still, Finding::Reenabled] {
                let task = desktop_card(&launch, finding, lang);
                assert!(task.select_only);
                assert!(!task.stop_confirmation);
                assert_eq!(task.native_request_id.as_deref(), Some("cursor-max:abc"));
                assert_eq!(task.questions.len(), 1);
                assert_eq!(task.questions[0].predefined_options.len(), 1);
                assert!(!task.message.text.contains("/max-mode"));
                assert!(task.message.text.contains("Max"));
            }
        }
    }

    #[test]
    fn the_card_asks_for_one_acknowledgement_and_is_not_a_stop_card() {
        let launch = Launch {
            id: "abc".into(),
            project: "/p/demo".into(),
            config: PathBuf::from("/c/cli-config.json"),
        };
        for lang in [Lang::Zh, Lang::En] {
            for finding in [Finding::Still, Finding::Reenabled] {
                let task = card(&launch, finding, lang);
                assert!(task.select_only);
                assert!(!task.stop_confirmation);
                assert_eq!(task.native_request_id.as_deref(), Some("cursor-max:abc"));
                assert_eq!(task.agent_session_id, None);
                assert_eq!(task.questions.len(), 1);
                assert_eq!(task.questions[0].predefined_options.len(), 1);
                assert_eq!(task.project, "/p/demo");
                assert!(task.message.text.contains("/max-mode"));
            }
        }
    }
}
