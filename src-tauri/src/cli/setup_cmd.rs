//! CLI `setup` subcommand: the guided setup after install (`docs/specs/onboarding.md` D1, D20).
//!
//! ```text
//! AskHuman setup [--text]   connect your Agents and, optionally, remote answering
//! ```
//!
//! The flow itself lives in `crate::onboarding`; this file parses the command line, decides
//! whether the terminal can run it, and owns the one-line welcome hint of the bare `AskHuman`.

use super::cfgio;
use super::help;
use super::print_line;
use crate::i18n::{self, Lang};
use std::io::{IsTerminal, Write};
use std::process::exit;

/// What the command line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the setup; `text` asks for the terminal version explicitly.
    Run {
        text: bool,
    },
    Help,
}

/// Why the command line was refused, before anything was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// An option or word the command does not take.
    Unknown(String),
}

/// Parse the arguments after `setup`.
pub fn parse(args: &[String]) -> Result<Command, ParseError> {
    let mut text = false;
    for arg in args {
        match arg.as_str() {
            "--text" => text = true,
            "help" | "-h" | "--help" => return Ok(Command::Help),
            other => return Err(ParseError::Unknown(other.to_string())),
        }
    }
    Ok(Command::Run { text })
}

pub fn usage(lang: Lang) -> String {
    let prog = help::program_name();
    match lang {
        Lang::En => [
            format!("Usage: {prog} setup [--text]"),
            String::new(),
            "Guided setup after install: connect your Agents (Cursor, Claude Code, Codex, ...) and,".to_string(),
            "optionally, remote answering. It takes about 2 minutes; run it again any time to change".to_string(),
            "things. What you already have is kept, only what you confirm is changed.".to_string(),
            String::new(),
            "Options:".to_string(),
            "  --text    Use the terminal version (the default when there is no graphical session)".to_string(),
            "  --help    Show this help".to_string(),
        ]
        .join("\n"),
        Lang::Zh => [
            format!("用法: {prog} setup [--text]"),
            String::new(),
            "安装后的引导设置：接入 Agent（Cursor、Claude Code、Codex 等），并可选配置远程回答。".to_string(),
            "约 2 分钟；之后随时可以重新运行来调整。已有的配置会保留，只改动你确认的项。".to_string(),
            String::new(),
            "选项:".to_string(),
            "  --text    使用终端版（没有图形会话时默认就是终端版）".to_string(),
            "  --help    显示此帮助".to_string(),
        ]
        .join("\n"),
    }
}

fn parse_error(error: &ParseError, lang: Lang) -> String {
    match error {
        ParseError::Unknown(arg) => cfgio::t(
            lang,
            &format!("setup does not take {arg}"),
            &format!("setup 不接受 {arg}"),
        ),
    }
}

/// Why the terminal flow cannot run, or `None` when it can: it needs somebody to answer, and a
/// pipe would only ever hit end of input.
pub fn text_unavailable(stdin_is_tty: bool, lang: Lang) -> Option<String> {
    (!stdin_is_tty).then(|| {
        cfgio::t(
            lang,
            "setup needs an interactive terminal, but stdin is not one. Run it in a terminal, or configure without prompts with `AskHuman agents`, `AskHuman channel` and `AskHuman config`.",
            "setup 需要交互式终端，但 stdin 不是终端。请在终端里运行；需要免交互配置时可用 `AskHuman agents`、`AskHuman channel`、`AskHuman config`。",
        )
    })
}

/// Where the setup runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The wizard window, through the unified GUI Host.
    Window,
    /// The terminal version.
    Terminal,
}

/// The terminal version when asked for (`--text`) or when no graphical session can show the
/// window; the window otherwise.
pub fn route(text: bool, gui_available: bool) -> Route {
    if text || !gui_available {
        Route::Terminal
    } else {
        Route::Window
    }
}

/// What to do once the window could not be opened (the GUI Host would not start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowFailure {
    /// A person at a terminal still gets the setup there, after this notice.
    ContinueInTerminal { notice: String },
    /// Nobody can answer a terminal flow: report the reason and exit 1.
    Fail { message: String },
}

pub fn window_failure(stdin_is_tty: bool, error: &str, lang: Lang) -> WindowFailure {
    if stdin_is_tty {
        WindowFailure::ContinueInTerminal {
            notice: cfgio::t(
                lang,
                &format!("Could not open the setup window ({error}); continuing in the terminal."),
                &format!("无法打开设置向导窗口（{error}），改在终端里继续。"),
            ),
        }
    } else {
        WindowFailure::Fail {
            message: cfgio::t(
                lang,
                &format!("could not open the setup window: {error}"),
                &format!("无法打开设置向导窗口：{error}"),
            ),
        }
    }
}

/// Entry point: runs the setup and exits with its result.
pub fn dispatch(args: &[String], lang: Lang) -> ! {
    match parse(args) {
        Ok(Command::Help) => {
            print_line(&usage(lang));
            exit(0);
        }
        Ok(Command::Run { text }) => run(text, lang),
        Err(error) => {
            eprintln!("{}{}\n", i18n::err_prefix(lang), parse_error(&error, lang));
            eprintln!("{}", usage(lang));
            exit(1);
        }
    }
}

fn run(text: bool, lang: Lang) -> ! {
    if route(text, super::agents_cmd::gui_available()) == Route::Window {
        // The window belongs to the GUI Host: ask it and return at once, like `--settings`.
        let error = match crate::gui_host::host_open(
            crate::gui_host::WindowKind::Setup,
            false,
            None,
            None,
            None,
        ) {
            Ok(()) => exit(0),
            Err(error) => error.to_string(),
        };
        match window_failure(cfgio::stdin_is_tty(), &error, lang) {
            WindowFailure::ContinueInTerminal { notice } => eprintln!("{notice}"),
            WindowFailure::Fail { message } => {
                eprintln!("{}{}", i18n::err_prefix(lang), message);
                exit(1);
            }
        }
    }
    if let Some(message) = text_unavailable(cfgio::stdin_is_tty(), lang) {
        eprintln!("{}{}", i18n::err_prefix(lang), message);
        exit(1);
    }
    exit(crate::onboarding::text::run_real(lang));
}

// ——— the welcome hint of the bare command ———

/// Whether the help the bare `AskHuman` prints should end with the setup hint: only for a person
/// at a terminal (stdin and stderr are both terminals, so never an Agent's pipe) whose Agent step
/// is still open. `agents_decided` reads the machine, so it runs only when the cheap checks pass.
pub fn welcome_due(
    stdin_is_tty: bool,
    stderr_is_tty: bool,
    agents_decided: impl FnOnce() -> bool,
) -> bool {
    stdin_is_tty && stderr_is_tty && !agents_decided()
}

/// The hint itself.
pub fn welcome_line(lang: Lang) -> String {
    i18n::tr(lang, "cli.setupWelcome").replace("{prog}", &help::program_name())
}

/// Print the hint on stderr when it is due. A failed write is ignored: it is only a hint.
pub fn print_welcome_if_due(lang: Lang) {
    let due = welcome_due(
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
        crate::onboarding::state::agents_decided,
    );
    if due {
        let _ = writeln!(std::io::stderr(), "\n{}", welcome_line(lang));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn the_command_takes_nothing_or_text_or_help() {
        assert_eq!(parse(&args(&[])), Ok(Command::Run { text: false }));
        assert_eq!(parse(&args(&["--text"])), Ok(Command::Run { text: true }));
        for help in ["help", "-h", "--help"] {
            assert_eq!(parse(&args(&[help])), Ok(Command::Help), "{help}");
        }
    }

    #[test]
    fn anything_else_is_refused_by_name() {
        for other in ["--yes", "--gui", "agents", "-t"] {
            assert_eq!(
                parse(&args(&[other])),
                Err(ParseError::Unknown(other.to_string())),
                "{other}"
            );
        }
        assert_eq!(
            parse(&args(&["--text", "extra"])),
            Err(ParseError::Unknown("extra".to_string()))
        );
        let message = parse_error(&ParseError::Unknown("--yes".to_string()), Lang::En);
        assert!(message.contains("--yes"), "{message}");
    }

    #[test]
    fn the_usage_documents_the_text_option_in_both_languages() {
        for lang in [Lang::En, Lang::Zh] {
            let text = usage(lang);
            assert!(text.contains("setup [--text]"), "{text}");
            assert!(text.contains("--help"), "{text}");
        }
    }

    #[test]
    fn the_window_is_the_default_and_the_terminal_the_fallback() {
        // (--text, a graphical session exists) → where the setup runs.
        for (text, gui, expected) in [
            (false, true, Route::Window),
            (true, true, Route::Terminal),
            (false, false, Route::Terminal),
            (true, false, Route::Terminal),
        ] {
            assert_eq!(route(text, gui), expected, "--text {text}, gui {gui}");
        }
    }

    #[test]
    fn a_person_at_a_terminal_continues_there_when_the_window_will_not_open() {
        match window_failure(true, "host did not answer", Lang::En) {
            WindowFailure::ContinueInTerminal { notice } => {
                assert!(notice.contains("host did not answer"), "{notice}");
                assert!(notice.contains("terminal"), "{notice}");
            }
            other => panic!("unexpected: {other:?}"),
        }
        match window_failure(true, "无响应", Lang::Zh) {
            WindowFailure::ContinueInTerminal { notice } => {
                assert!(
                    notice.contains("无响应") && notice.contains("终端"),
                    "{notice}"
                )
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn anyone_else_gets_the_reason_and_a_failing_exit() {
        match window_failure(false, "host did not answer", Lang::En) {
            WindowFailure::Fail { message } => {
                assert!(message.contains("host did not answer"), "{message}");
                assert!(message.contains("setup window"), "{message}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn the_terminal_flow_needs_a_terminal_to_answer_it() {
        assert_eq!(text_unavailable(true, Lang::En), None);
        let message = text_unavailable(false, Lang::En).expect("a pipe cannot answer");
        assert!(message.contains("interactive terminal"), "{message}");
        assert!(message.contains("AskHuman channel"), "{message}");
        let message = text_unavailable(false, Lang::Zh).expect("a pipe cannot answer");
        assert!(message.contains("交互式终端"), "{message}");
    }

    #[test]
    fn the_welcome_hint_is_for_a_person_at_a_terminal_with_the_agent_step_open() {
        // (stdin is a terminal, stderr is a terminal, Agent step decided) → shown?
        for (stdin, stderr, decided, shown) in [
            (true, true, false, true),
            (true, true, true, false),
            (false, true, false, false),
            (true, false, false, false),
            (false, false, false, false),
        ] {
            assert_eq!(
                welcome_due(stdin, stderr, || decided),
                shown,
                "stdin tty {stdin}, stderr tty {stderr}, decided {decided}"
            );
        }
    }

    #[test]
    fn the_machine_is_read_only_when_the_cheap_checks_pass() {
        let reads = Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            false
        };
        assert!(!welcome_due(false, true, read));
        assert!(!welcome_due(true, false, read));
        assert_eq!(
            reads.get(),
            0,
            "an Agent's pipe never reads the machine state"
        );
        assert!(welcome_due(true, true, read));
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn the_welcome_line_names_the_command_to_run() {
        for lang in [Lang::En, Lang::Zh] {
            let line = welcome_line(lang);
            assert!(line.contains(" setup"), "{line}");
            assert!(!line.contains("{prog}"), "{line}");
        }
    }
}
