//! CLI `away` subcommand (`docs/specs/special-replies.md` D40, N6):
//!
//! ```text
//! AskHuman away on [note…] [--session]   turn away mode on (everyone, or only this Agent session)
//! AskHuman away off [--session]          turn it off again (refused inside an Agent)
//! AskHuman away status                   what is set right now (reads the state file, no daemon)
//! ```

use super::cfgio;
use super::print_line;
use crate::daemon::away::{AwayOp, AwayState, SessionMode, SessionSetting};
use crate::i18n::{self, Lang};
use std::process::exit;

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    On { note: Option<String>, session: bool },
    Off { session: bool },
    Status,
}

/// Why the command line was refused, before anything was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnknownSubcommand(String),
    UnknownOption(String),
    /// `status` takes no arguments; `off` takes no note.
    UnexpectedText(String),
}

/// Parse the arguments after `away`. Flags may sit anywhere; the words that are not flags form the
/// note of `on` (so it needs no quoting).
pub fn parse(args: &[String]) -> Result<Command, ParseError> {
    let mut words: Vec<&str> = Vec::new();
    let mut session = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--session" | "-s" => session = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(ParseError::UnknownOption(flag.to_string()));
            }
            word => words.push(word),
        }
    }
    let text = words.join(" ");
    match args.first().map(String::as_str) {
        Some("on") => Ok(Command::On {
            note: Some(text.trim().to_string()).filter(|note| !note.is_empty()),
            session,
        }),
        Some("off") if text.is_empty() => Ok(Command::Off { session }),
        Some("status") | None if text.is_empty() && !session => Ok(Command::Status),
        Some("off" | "status") | None => Err(ParseError::UnexpectedText(text)),
        Some(other) => Err(ParseError::UnknownSubcommand(other.to_string())),
    }
}

/// The key of the Agent session this process runs in (the same key the daemon derives for the
/// session's questions); `None` outside an Agent or when the session cannot be told.
fn caller_session_key() -> Option<String> {
    let context = super::caller_context();
    context
        .agent_session_id
        .as_deref()
        .map(crate::daemon::away::sid_key)
        .or_else(|| {
            context
                .mcp_instance_id
                .as_deref()
                .map(|id| format!("mcp:{id}"))
        })
}

/// Whether this process was started by an Agent. An Agent must not turn away mode off itself: the
/// user decides when they are back (N6).
fn invoked_by_agent() -> bool {
    super::caller_agent_hint()
        .or_else(crate::agents::detect::detect_invoking_agent)
        .is_some()
}

/// The operation a command asks the daemon for, or why it cannot be done from here.
pub fn operation(
    command: &Command,
    session_key: Option<&str>,
    by_agent: bool,
    lang: Lang,
) -> Result<AwayOp, String> {
    let need_session = || {
        session_key.map(str::to_string).ok_or_else(|| {
            cfgio::t(
                lang,
                "--session works inside an Agent session only (this process has none)",
                "--session 只能在 Agent 会话里使用（当前进程没有会话）",
            )
        })
    };
    match command {
        Command::On {
            note,
            session: false,
        } => Ok(AwayOp::Start { note: note.clone() }),
        Command::On {
            note,
            session: true,
        } => Ok(AwayOp::Session {
            session_key: need_session()?,
            setting: SessionSetting::Away,
            note: note.clone(),
        }),
        Command::Off { .. } if by_agent => Err(cfgio::t(
            lang,
            "away mode can only be turned off by the user, not from inside an Agent",
            "离开模式只能由用户关闭，不能在 Agent 内关闭",
        )),
        Command::Off { session: false } => Ok(AwayOp::End),
        Command::Off { session: true } => Ok(AwayOp::Session {
            session_key: need_session()?,
            setting: SessionSetting::Follow,
            note: None,
        }),
        Command::Status => Ok(AwayOp::State),
    }
}

/// `HH:MM` of a millisecond timestamp in local time.
fn clock(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// How long ago, to the minute.
fn elapsed(since_ms: u64, now_ms: u64, lang: Lang) -> String {
    let minutes = now_ms.saturating_sub(since_ms) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => cfgio::t(lang, "under a minute", "不到 1 分钟"),
        (0, m) => cfgio::t(lang, &format!("{m} min"), &format!("{m} 分钟")),
        (h, m) => cfgio::t(
            lang,
            &format!("{h} h {m} min"),
            &format!("{h} 小时 {m} 分钟"),
        ),
    }
}

/// What `status` prints: the global switch, then the sessions that were set on their own, then the
/// calling session when there is one.
pub fn status_lines(
    state: &AwayState,
    session_key: Option<&str>,
    now_ms: u64,
    lang: Lang,
) -> Vec<String> {
    let mut lines = Vec::new();
    match &state.global {
        Some(global) => {
            lines.push(cfgio::t(
                lang,
                &format!(
                    "Away mode: on since {} ({}), {} auto-replies",
                    clock(global.since_ms),
                    elapsed(global.since_ms, now_ms, lang),
                    global.replies
                ),
                &format!(
                    "离开模式：开启中，{} 开始（已 {}），自动回复 {} 次",
                    clock(global.since_ms),
                    elapsed(global.since_ms, now_ms, lang),
                    global.replies
                ),
            ));
            if let Some(note) = &global.note {
                lines.push(cfgio::t(
                    lang,
                    &format!("Note: {note}"),
                    &format!("备注：{note}"),
                ));
            }
        }
        None => lines.push(cfgio::t(lang, "Away mode: off", "离开模式：未开启")),
    }
    let count = |mode: SessionMode| state.sessions.values().filter(|e| e.mode == mode).count();
    let (away, mute) = (count(SessionMode::Away), count(SessionMode::Mute));
    if away + mute > 0 {
        lines.push(cfgio::t(
            lang,
            &format!("Sessions set on their own: {away} away, {mute} not using AskHuman"),
            &format!("单独设置的会话：{away} 个离开，{mute} 个不再使用 AskHuman"),
        ));
    }
    if let Some(key) = session_key {
        let own = state.sessions.get(key).map(|e| e.mode);
        let text = match (own, state.global.is_some()) {
            (Some(SessionMode::Away), _) => cfgio::t(
                lang,
                "away (set for this session)",
                "离开（本会话单独设置）",
            ),
            (Some(SessionMode::Mute), _) => cfgio::t(
                lang,
                "not using AskHuman (set for this session)",
                "不再使用 AskHuman（本会话单独设置）",
            ),
            (Some(SessionMode::Normal), _) => cfgio::t(
                lang,
                "asks normally (set for this session)",
                "照常提问（本会话单独设置）",
            ),
            (None, true) => cfgio::t(
                lang,
                "away (follows the global setting)",
                "离开（跟随全局）",
            ),
            (None, false) => cfgio::t(lang, "normal", "正常"),
        };
        lines.push(cfgio::t(
            lang,
            &format!("This session: {text}"),
            &format!("本会话：{text}"),
        ));
    }
    lines
}

fn now_ms() -> u64 {
    crate::history::now_ms().max(0) as u64
}

/// What a successful `on` / `off` prints.
fn done_line(command: &Command, lang: Lang) -> String {
    match command {
        Command::On { session: false, .. } => cfgio::t(lang, "Away mode is on", "离开模式已开启"),
        Command::On { session: true, .. } => cfgio::t(
            lang,
            "Away mode is on for this session",
            "本会话的离开模式已开启",
        ),
        Command::Off { session: false } => cfgio::t(lang, "Away mode is off", "离开模式已关闭"),
        Command::Off { session: true } => cfgio::t(
            lang,
            "This session follows the global setting again",
            "本会话已恢复跟随全局设置",
        ),
        Command::Status => String::new(),
    }
}

fn parse_error(error: &ParseError, lang: Lang) -> String {
    match error {
        ParseError::UnknownSubcommand(sub) => cfgio::t(
            lang,
            &format!("unknown away subcommand: {sub} (use on / off / status)"),
            &format!("未知的 away 子命令: {sub}（可用 on / off / status）"),
        ),
        ParseError::UnknownOption(option) => cfgio::t(
            lang,
            &format!("unknown option: {option} (the only option is --session)"),
            &format!("未知选项: {option}（只有 --session）"),
        ),
        ParseError::UnexpectedText(text) => cfgio::t(
            lang,
            &format!("unexpected argument: {text}"),
            &format!("多余的参数: {text}"),
        ),
    }
}

/// `AskHuman away …` entry point (`args` excludes "away" itself).
pub fn dispatch(args: &[String], lang: Lang) -> ! {
    let fail = |message: String| -> ! {
        eprintln!("{}{}", i18n::err_prefix(lang), message);
        exit(1);
    };
    let command = parse(args).unwrap_or_else(|error| fail(parse_error(&error, lang)));
    let session_key = caller_session_key();
    if command == Command::Status {
        // The state file is the daemon's own record; reading it works with the daemon stopped.
        let state = crate::daemon::away::read_state_file_at(&crate::paths::away_file());
        for line in status_lines(&state, session_key.as_deref(), now_ms(), lang) {
            print_line(&line);
        }
        exit(0);
    }
    let op = operation(&command, session_key.as_deref(), invoked_by_agent(), lang)
        .unwrap_or_else(|message| fail(message));
    match cfgio::block_on(crate::client::away_op(op)) {
        Ok(_) => {
            print_line(&done_line(&command, lang));
            exit(0);
        }
        Err(error) => fail(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::away::{GlobalAway, SessionEntry};
    use std::collections::BTreeMap;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn on_takes_the_rest_as_the_note() {
        assert_eq!(
            parse(&args(&["on"])),
            Ok(Command::On {
                note: None,
                session: false
            })
        );
        assert_eq!(
            parse(&args(&["on", "don't", "touch", "production"])),
            Ok(Command::On {
                note: Some("don't touch production".into()),
                session: false
            })
        );
        // The flag may come before, between or after the note.
        for list in [
            ["on", "--session", "keep", "going"],
            ["on", "keep", "--session", "going"],
            ["on", "keep", "going", "--session"],
        ] {
            assert_eq!(
                parse(&args(&list)),
                Ok(Command::On {
                    note: Some("keep going".into()),
                    session: true
                })
            );
        }
        // A blank note is no note.
        assert_eq!(
            parse(&args(&["on", "  "])),
            Ok(Command::On {
                note: None,
                session: false
            })
        );
    }

    #[test]
    fn off_and_status_take_no_text() {
        assert_eq!(parse(&args(&["off"])), Ok(Command::Off { session: false }));
        assert_eq!(
            parse(&args(&["off", "-s"])),
            Ok(Command::Off { session: true })
        );
        assert_eq!(parse(&args(&["status"])), Ok(Command::Status));
        // No subcommand shows the status, the cheapest thing to do.
        assert_eq!(parse(&[]), Ok(Command::Status));
        assert_eq!(
            parse(&args(&["off", "now"])),
            Err(ParseError::UnexpectedText("now".into()))
        );
        assert_eq!(
            parse(&args(&["status", "--session"])),
            Err(ParseError::UnexpectedText(String::new()))
        );
    }

    #[test]
    fn unknown_input_is_refused() {
        assert_eq!(
            parse(&args(&["pause"])),
            Err(ParseError::UnknownSubcommand("pause".into()))
        );
        assert_eq!(
            parse(&args(&["on", "--for", "2h"])),
            Err(ParseError::UnknownOption("--for".into()))
        );
    }

    #[test]
    fn an_agent_may_turn_away_mode_on_but_never_off() {
        let on = Command::On {
            note: Some("n".into()),
            session: false,
        };
        assert_eq!(
            operation(&on, Some("sid:a"), true, Lang::En),
            Ok(AwayOp::Start {
                note: Some("n".into())
            })
        );
        assert!(operation(&Command::Off { session: false }, None, true, Lang::En).is_err());
        assert!(operation(
            &Command::Off { session: true },
            Some("sid:a"),
            true,
            Lang::En
        )
        .is_err());
        assert_eq!(
            operation(&Command::Off { session: false }, None, false, Lang::En),
            Ok(AwayOp::End)
        );
        assert_eq!(
            operation(&Command::Status, None, true, Lang::En),
            Ok(AwayOp::State)
        );
    }

    #[test]
    fn session_scope_needs_a_session() {
        let on = Command::On {
            note: None,
            session: true,
        };
        assert!(operation(&on, None, false, Lang::En).is_err());
        assert_eq!(
            operation(&on, Some("sid:a"), true, Lang::En),
            Ok(AwayOp::Session {
                session_key: "sid:a".into(),
                setting: SessionSetting::Away,
                note: None
            })
        );
        assert_eq!(
            operation(
                &Command::Off { session: true },
                Some("sid:a"),
                false,
                Lang::En
            ),
            Ok(AwayOp::Session {
                session_key: "sid:a".into(),
                setting: SessionSetting::Follow,
                note: None
            })
        );
    }

    fn state(global: bool, sessions: &[(&str, SessionMode)]) -> AwayState {
        AwayState {
            version: 1,
            global: global.then(|| GlobalAway {
                since_ms: 0,
                note: Some("not prod".into()),
                replies: 6,
                replies_by_session: BTreeMap::new(),
            }),
            sessions: sessions
                .iter()
                .map(|(key, mode)| {
                    (
                        key.to_string(),
                        SessionEntry {
                            mode: *mode,
                            since_ms: 0,
                            note: None,
                            replies: 0,
                            last_seen_ms: 0,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn status_reports_the_switch_the_note_and_the_sessions() {
        let now = 83 * 60_000;
        let lines = status_lines(
            &state(
                true,
                &[("sid:a", SessionMode::Away), ("sid:b", SessionMode::Mute)],
            ),
            Some("sid:b"),
            now,
            Lang::En,
        );
        assert!(lines[0].starts_with("Away mode: on since "));
        assert!(lines[0].contains("1 h 23 min"));
        assert!(lines[0].ends_with("6 auto-replies"));
        assert_eq!(lines[1], "Note: not prod");
        assert_eq!(
            lines[2],
            "Sessions set on their own: 1 away, 1 not using AskHuman"
        );
        assert_eq!(
            lines[3],
            "This session: not using AskHuman (set for this session)"
        );
    }

    #[test]
    fn status_when_nothing_is_set() {
        assert_eq!(
            status_lines(&AwayState::default(), None, 0, Lang::En),
            vec!["Away mode: off"]
        );
        assert_eq!(
            status_lines(&AwayState::default(), Some("sid:a"), 0, Lang::En),
            vec!["Away mode: off", "This session: normal"]
        );
        assert_eq!(
            status_lines(&state(true, &[]), Some("sid:a"), 0, Lang::Zh)
                .last()
                .map(String::as_str),
            Some("本会话：离开（跟随全局）")
        );
    }
}
