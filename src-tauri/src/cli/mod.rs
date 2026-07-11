pub mod agents_cmd;
pub mod args;
pub mod cfgio;
pub mod channel_cmd;
pub mod config_cmd;
pub mod debug_cmd;
pub mod dev_cmd;
pub mod doctor;
pub mod file_attachment;
pub mod help;
pub mod image_writer;
pub mod output;

use crate::i18n::{self, Lang};
use std::process::exit;

/// 向 stdout 输出一行文本，并把 BrokenPipe（读端提前关闭，如 `AskHuman --agent-help | head`）
/// 视为正常结束：写失败一律静默忽略，退出码由调用方决定（纯输出命令随后 exit(0)，错误分支 exit(1)）。
///
/// 背景：Rust 运行时默认把 SIGPIPE 设为忽略，写已关闭管道返回 EPIPE 而非被信号终止；若用
/// `println!`，写失败会 panic，而 release 为 `panic = "abort"`，最终以退出码 134 退出。改用本函数规避。
fn print_line(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{text}").and_then(|_| out.flush());
}

/// 入口分发：在创建任何窗口前按 argv 分流。
pub fn dispatch() {
    // Dev Instance: pin ASKHUMAN_HOME / re-exec worktree bin before any config or GUI load.
    crate::dev_instance::maybe_enter_dev_instance();

    let argv: Vec<String> = std::env::args().collect();
    let lang = Lang::current();

    // 完全无参数：报错 + 通用 Help（让用户直接 `AskHuman` 即可看到全部用法，而非仅提问说明）。
    // 注意：有参数但解析失败 / 未知选项的情况，仍展示提问导向的 agent-help（见下方分支）。
    if argv.len() < 2 {
        eprintln!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "cli.missingContent")
        );
        eprintln!(
            "{}\n",
            i18n::tr(lang, "cli.seeAgentHelp").replace("{prog}", &help::program_name())
        );
        print_line(&help::help_text(lang));
        exit(1);
    }

    match argv[1].as_str() {
        "--help" | "-h" => {
            print_line(&help::help_text(lang));
            exit(0);
        }
        "--version" | "-v" => {
            print_line(&help::version_text());
            exit(0);
        }
        "--agent-help" => {
            print_line(&help::agent_help_text(lang));
            exit(0);
        }
        "--scripting-help" => {
            print_line(&help::scripting_help_text(lang));
            exit(0);
        }
        // 设置/历史窗口只需 general(主题)；密钥的「已保存」判定由前端 `get_settings` 单独读取。
        // 故用 load_without_secrets()，避免打开这两个窗口时无谓读钥匙串。
        // unix：彻底路由到统一 GUI 宿主（全局单窗，spec D3）——宿主在则聚焦/新建、不在则拉起；
        // 失败（极端：宿主起不来）兜底本进程直接建窗，保证窗口至少能打开。
        "--settings" => {
            #[cfg(unix)]
            {
                if crate::gui_host::host_open(
                    crate::gui_host::WindowKind::Settings,
                    false,
                    None,
                    None,
                )
                .is_ok()
                {
                    exit(0);
                }
            }
            crate::app::run_settings(crate::config::AppConfig::load_without_secrets());
        }
        // 独立历史窗口：默认当前项目（向上找 .git 根、回退 cwd）；`--all` 默认展示全部项目。
        "--history" => {
            let all = argv[2..].iter().any(|a| a == "--all");
            #[cfg(unix)]
            {
                // 项目过滤随请求经宿主 IPC 传递（宿主自身 cwd 无意义）。
                let project = crate::project::detect();
                if crate::gui_host::host_open(
                    crate::gui_host::WindowKind::History,
                    all,
                    Some(project),
                    None,
                )
                .is_ok()
                {
                    exit(0);
                }
            }
            crate::app::run_history(
                crate::project::detect(),
                all,
                crate::config::AppConfig::load_without_secrets(),
            );
        }
        // 隐藏的统一 GUI 宿主角色（spec D2）：单实例托盘 + 设置/历史/Agent 窗口宿主。
        // 由 CLI 路由 / daemon 按需 spawn；抢宿主单实例锁失败即直接退出（已有宿主在跑）。
        "--gui-host" => {
            #[cfg(unix)]
            {
                crate::app::run_gui_host(crate::config::AppConfig::load_without_secrets());
            }
            #[cfg(not(unix))]
            {
                eprintln!("--gui-host is not supported on this platform");
                exit(1);
            }
        }
        // 隐藏的 GUI Helper 角色：由 Daemon spawn（`--popup --endpoint <sock> --token <tok>`）。
        "--popup" => {
            #[cfg(unix)]
            {
                let mut endpoint = String::new();
                let mut token = String::new();
                // 方案6：预热模式由 daemon 以 `--popup --warm` 拉起（无 token），先建窗挂载、隐藏待命，
                // 入 daemon「热池」，来请求时由 daemon 喂 Show 领用上屏。
                let mut warm = false;
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
                        "--warm" => {
                            warm = true;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
                crate::app::run_gui_helper(endpoint, token, warm);
            }
            #[cfg(not(unix))]
            {
                eprintln!("--popup is not supported on this platform");
                exit(1);
            }
        }
        // Dev Instance：enable/disable/status/preset（多 WorkTree 并行开发隔离）。
        "dev" => {
            dev_cmd::dispatch(&argv[2..], lang);
        }
        // 常驻 Daemon 管理子命令：AskHuman daemon <run|start|stop|restart|status|logs>。
        // 极端歧义（问题正好是 "daemon"）可用 `AskHuman -q daemon` 规避。
        "daemon" => {
            crate::daemon::dispatch(&argv[2..]);
        }
        // MCP server 角色：以 STDIO 暴露 `ask` 工具，供 Codex / Claude Code / Cursor 等 MCP 客户端调用。
        // 每次工具调用都 spawn 一个 `AskHuman --output json …` 子进程复用既有 ask 流程（见 mcp 模块）。
        // 极端歧义（问题正好是 "mcp"）可用 `AskHuman -q mcp` 规避。
        "mcp" => {
            crate::mcp::run();
        }
        // 隐藏的生命周期上报器：由三家 Agent 的用户级 hook 调用
        // （`AskHuman __agent-hook <agent> <event>`，spec D20）。即发即走、静默退出。
        "__agent-hook" => {
            #[cfg(unix)]
            {
                crate::agents::report::run(&argv[2..]);
            }
            exit(0);
        }
        // 隐藏权限 Hook：`AskHuman __permission-hook <claude|codex>`。
        // 从 stdin 读 PermissionRequest JSON，经 daemon 取得人的决定，stdout 输出 allow/deny JSON。
        // 基础设施失败时 stdout 为空（Agent 回原生弹窗）。
        "__permission-hook" => {
            #[cfg(unix)]
            {
                let code = crate::permission::run_hook(&argv[2..]);
                exit(code);
            }
            #[cfg(not(unix))]
            {
                exit(0);
            }
        }
        // Agent 状态 + 集成子命令组（spec：cli-config）：monitor / show / install / uninstall / update。
        "agents" => {
            agents_cmd::dispatch(&argv[2..], lang);
        }
        // IM 渠道配置（headless / 无 GUI）：list / set / enable / disable / test / detect。
        "channel" => {
            channel_cmd::dispatch(&argv[2..], lang);
        }
        // 通用键值兜底：show / get / set / unset / path。
        "config" => {
            config_cmd::dispatch(&argv[2..], lang);
        }
        // 一屏体检：daemon / 渠道 / 集成。
        "doctor" => {
            doctor::dispatch(&argv[2..], lang);
            exit(0);
        }
        // 隐藏调试子命令组（不进 help）：如钉钉 watch PoC 探针 `debug dd-watch-poc`。
        "debug" => {
            debug_cmd::dispatch(&argv[2..], lang);
            exit(0);
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
                        | "--stdin"
                        | "--select-only"
                        | "--single"
                        | "--output"
                ) =>
        {
            eprintln!(
                "{}{}\n",
                i18n::err_prefix(lang),
                i18n::tr(lang, "cli.unknownOption").replace("{opt}", first)
            );
            print_line(&help::agent_help_text(lang));
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
                    // 性能埋点（spec popup-launch-performance §7）：仅 `ASKHUMAN_PERF` 开启时铸 id，
                    // 经 TaskRequest 透传到 daemon/helper/前端串联整条时间线；关闭则恒空、零开销。
                    let perf_id = if crate::perf::enabled() {
                        format!("{}-{}", std::process::id(), crate::perf::now_ms())
                    } else {
                        String::new()
                    };
                    crate::perf::mark_at(&perf_id, "cli.start", crate::perf::start_ms());
                    // harness 注入的 spawn 时刻（含进程创建 / 加载，main 之前不可见的开销）。
                    crate::perf::mark_spawn(&perf_id);
                    // 顺带探测调用方 Agent 身份（生命周期追踪 spec D21）：仅 env 读取（家族 + 会话 ID，零 ps）。
                    // 方案5(b)：进程树 walk（数十 ms 的 ps 游走）移到 daemon 异步进行——这里只带 CLI 自身 pid。
                    let (agent_kind, agent_session_id) = detect_caller_agent();
                    crate::perf::mark(&perf_id, "cli.detect_done");
                    // 来源名解析：未定制 `ASKHUMAN_ENV_SOURCE_NAME` 时，用探测到的 Agent 名
                    // （Claude Code / Codex / Cursor）替代默认 "the Loop"；供渠道消息头 + 历史共用
                    // （弹窗标题另由前端按胶囊内联渲染）。MCP 模式 env 判不出家族 → 回退 "the Loop"。
                    let resolved_agent_kind = agent_kind
                        .as_deref()
                        .and_then(crate::agents::AgentKind::parse);
                    let task = crate::ipc::TaskRequest {
                        message,
                        questions,
                        // Markdown 渲染恒开（`--no-markdown` 已移除）；弹窗内可临时切换为源码视图。
                        is_markdown: true,
                        source: crate::models::source_name_for_agent(resolved_agent_kind),
                        lang: lang.code().to_string(),
                        project: crate::project::detect(),
                        select_only: parsed.select_only,
                        single: parsed.single,
                        output_format: parsed.output_format,
                        agent_kind,
                        agent_session_id,
                        agent_pid: None,
                        caller_pid: std::process::id(),
                        from_mcp: from_mcp_env(),
                        perf_id,
                        perf_autodismiss: crate::perf::autodismiss(),
                    };
                    crate::client::run_ask(task);
                }
                // 非 unix：暂无 Daemon，沿用单进程内运行（Windows named pipe 待后续 Phase）。
                #[cfg(not(unix))]
                {
                    let mut request = crate::models::AskRequest::new(message, questions, true);
                    request.select_only = parsed.select_only;
                    request.single = parsed.single;
                    request.output_format = parsed.output_format;
                    crate::app::run_ask(request, crate::config::AppConfig::load());
                }
            }
            Err(e) => {
                eprintln!("{}{}\n", i18n::err_prefix(lang), e);
                print_line(&help::agent_help_text(lang));
                exit(1);
            }
        },
    }
}

/// 探测发起 `AskHuman` 调用的 Agent 身份的**快速部分**（家族 + 会话 ID，仅读 env，零 ps）。
/// 方案5(b)：进程树 walk（拿 agent pid，数十 ms 的 ps 游走）不在此做——改由 daemon 从 `caller_pid`
/// 异步进行（含 env 判不出家族的 **MCP 兜底**：daemon walk_any_agent）。env 判不出则两者皆 None。
#[cfg(unix)]
fn detect_caller_agent() -> (Option<String>, Option<String>) {
    use crate::agents::detect;
    if let Some(kind) = detect::detect_running_agent() {
        let sid = detect::session_id_from_env(kind);
        return (Some(kind.as_str().to_string()), sid);
    }
    (None, None)
}

/// 是否经 MCP 模式发起（`AskHuman mcp` spawn 子进程时设 env `ASKHUMAN_FROM_MCP`）。
/// 非空且非 `0` 即视为真（沿用本项目 env 开关惯例）。daemon 据此对该次 ask「只刷新、不新建」session。
#[cfg(unix)]
fn from_mcp_env() -> bool {
    std::env::var("ASKHUMAN_FROM_MCP")
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0"
        })
        .unwrap_or(false)
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
        eprintln!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "cli.stdinIsTty")
        );
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
