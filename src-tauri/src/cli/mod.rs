pub mod args;
pub mod file_attachment;
pub mod help;
pub mod image_writer;
pub mod output;

use crate::i18n::{self, Lang};
use std::process::exit;

/// 入口分发：在创建任何窗口前按 argv 分流。
pub fn dispatch() {
    let argv: Vec<String> = std::env::args().collect();
    let lang = Lang::current();

    // 完全无参数：报错 + 通用 Help（让用户直接 `AskHuman` 即可看到全部用法，而非仅提问说明）。
    // 注意：有参数但解析失败 / 未知选项的情况，仍展示提问导向的 agent-help（见下方分支）。
    if argv.len() < 2 {
        eprintln!("{}{}", i18n::err_prefix(lang), i18n::tr(lang, "cli.missingContent"));
        eprintln!(
            "{}\n",
            i18n::tr(lang, "cli.seeAgentHelp").replace("{prog}", &help::program_name())
        );
        println!("{}", help::help_text(lang));
        exit(1);
    }

    match argv[1].as_str() {
        "--help" | "-h" => {
            println!("{}", help::help_text(lang));
            exit(0);
        }
        "--version" | "-v" => {
            println!("{}", help::version_text());
            exit(0);
        }
        "--agent-help" => {
            println!("{}", help::agent_help_text(lang));
            exit(0);
        }
        // 设置/历史窗口只需 general(主题)；密钥的「已保存」判定由前端 `get_settings` 单独读取。
        // 故用 load_without_secrets()，避免打开这两个窗口时无谓读钥匙串。
        "--settings" => {
            crate::app::run_settings(crate::config::AppConfig::load_without_secrets());
        }
        // 独立历史窗口：默认当前项目（向上找 .git 根、回退 cwd）；`--all` 默认展示全部项目。
        "--history" => {
            let all = argv[2..].iter().any(|a| a == "--all");
            crate::app::run_history(
                crate::project::detect(),
                all,
                crate::config::AppConfig::load_without_secrets(),
            );
        }
        // 隐藏的 GUI Helper 角色：由 Daemon spawn（`--popup --endpoint <sock> --token <tok>`）。
        "--popup" => {
            #[cfg(unix)]
            {
                let mut endpoint = String::new();
                let mut token = String::new();
                let mut i = 2;
                while i < argv.len() {
                    match argv[i].as_str() {
                        "--endpoint" if i + 1 < argv.len() => {
                            endpoint = argv[i + 1].clone();
                            i += 2;
                        }
                        "--token" if i + 1 < argv.len() => {
                            token = argv[i + 1].clone();
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
                crate::app::run_gui_helper(endpoint, token);
            }
            #[cfg(not(unix))]
            {
                eprintln!("--popup is not supported on this platform");
                exit(1);
            }
        }
        // 常驻 Daemon 管理子命令：AskHuman daemon <run|start|stop|restart|status|logs>。
        // 极端歧义（问题正好是 "daemon"）可用 `AskHuman -q daemon` 规避。
        "daemon" => {
            crate::daemon::dispatch(&argv[2..]);
        }
        // 第一题既可用位置参数，也可用 `-q`/`--question`；提问相关 flag 一律进入提问分支，
        // 由 `parse_ask` 给出精确错误（如缺少问题内容、选项需在问题之后）。
        first
            if first.starts_with('-')
                && !matches!(
                    first,
                    "-q" | "--question"
                        | "-o"
                        | "--option"
                        | "-o!"
                        | "--option!"
                        | "-f"
                        | "--file"
                        | "--no-markdown"
                        | "--stdin"
                ) =>
        {
            eprintln!(
                "{}{}\n",
                i18n::err_prefix(lang),
                i18n::tr(lang, "cli.unknownOption").replace("{opt}", first)
            );
            println!("{}", help::agent_help_text(lang));
            exit(1);
        }
        _ => match parse_ask_with_stdin(&argv[1..], lang) {
            Ok(parsed) => {
                // 解析 Message 的展示附件（-f 始终归 Message）。
                let files = match file_attachment::resolve(&parsed.message_files, lang) {
                    Ok(files) => files,
                    Err(e) => {
                        eprintln!("{}{}", i18n::err_prefix(lang), e);
                        exit(1);
                    }
                };
                let message = crate::models::MessagePrompt::new(parsed.message_text, files);
                let questions: Vec<crate::models::Question> = parsed
                    .questions
                    .into_iter()
                    .map(|q| {
                        let options = q
                            .options
                            .into_iter()
                            .map(|o| crate::models::OptionItem::new(o.text, o.recommended))
                            .collect();
                        crate::models::Question::new(q.message, options)
                    })
                    .collect();
                // unix：瘦客户端经 Daemon + GUI Helper（A11：上送 source name 与解析好的 lang）。
                #[cfg(unix)]
                {
                    let task = crate::ipc::TaskRequest {
                        message,
                        questions,
                        is_markdown: parsed.is_markdown,
                        source: crate::models::source_name(),
                        lang: lang.code().to_string(),
                        project: crate::project::detect(),
                    };
                    crate::client::run_ask(task);
                }
                // 非 unix：暂无 Daemon，沿用单进程内运行（Windows named pipe 待后续 Phase）。
                #[cfg(not(unix))]
                {
                    let request =
                        crate::models::AskRequest::new(message, questions, parsed.is_markdown);
                    crate::app::run_ask(request, crate::config::AppConfig::load());
                }
            }
            Err(e) => {
                eprintln!("{}{}\n", i18n::err_prefix(lang), e);
                println!("{}", help::agent_help_text(lang));
                exit(1);
            }
        },
    }
}

/// 提问解析的入口包装：仅当出现 `--stdin` 时读取标准输入作为 Message，
/// 再交给纯函数 `args::parse_ask`（stdin 内容以参数注入，保持其无 IO 副作用）。
fn parse_ask_with_stdin(args: &[String], lang: Lang) -> Result<args::AskArgs, String> {
    let stdin_message = if args.iter().any(|a| a == "--stdin") {
        Some(read_stdin_message(lang))
    } else {
        None
    };
    args::parse_ask(args, lang, stdin_message)
}

/// 读取标准输入作为 Message 文本（`--stdin`）。
///
/// - stdin 为终端（无管道输入）时不阻塞等待，直接报错退出，避免挂起；
/// - 读取失败时报错退出；
/// - 去除结尾的一个换行（`\n` 或 `\r\n`，即 heredoc 末尾的固有换行），
///   其余（含前导/内部空白）原样保留。
fn read_stdin_message(lang: Lang) -> String {
    use std::io::{IsTerminal, Read};
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprintln!("{}{}", i18n::err_prefix(lang), i18n::tr(lang, "cli.stdinIsTty"));
        exit(1);
    }
    let mut buf = String::new();
    if let Err(e) = stdin.read_to_string(&mut buf) {
        eprintln!("{}{}", i18n::err_prefix(lang), e);
        exit(1);
    }
    if let Some(stripped) = buf.strip_suffix('\n') {
        buf.truncate(stripped.len());
        if let Some(stripped) = buf.strip_suffix('\r') {
            buf.truncate(stripped.len());
        }
    }
    buf
}
