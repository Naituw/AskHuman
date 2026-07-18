//! CLI `todo` 子命令（spec todo-whats-next D6）：add / list / rm / clear。
//!
//! 项目 key 取调用 cwd 的 git 根（`project::detect`，与回复历史同规则）；
//! 存储直读直写 `todos.json`（D1 第 9 轮定案，不依赖 daemon 存活，跨平台同一套代码）。

use super::print_line;
use crate::i18n::{self, Lang};
use std::process::exit;

/// `AskHuman todo <add|list|rm|clear> …` 入口（args 不含 "todo" 本身）。
pub fn dispatch(args: &[String], lang: Lang) -> ! {
    let project = crate::project::detect();
    if project.is_empty() {
        // cwd 都取不到（极端环境）：无法归属项目。
        eprintln!("{}cannot determine project", i18n::err_prefix(lang));
        exit(1);
    }
    match args.first().map(String::as_str) {
        Some("add") => add(&project, &args[1..], lang),
        // 无子命令时默认 list（顺手查看）。
        Some("list") | None => list(&project, lang),
        Some("rm") => rm(&project, &args[1..], lang),
        Some("clear") => clear(&project, &args[1..], lang),
        Some(other) => {
            eprintln!(
                "{}{}",
                i18n::err_prefix(lang),
                i18n::tr(lang, "todo.unknownSubcommand").replace("{cmd}", other)
            );
            exit(1);
        }
    }
}

fn add(project: &str, args: &[String], lang: Lang) -> ! {
    // `--auto`（第 17 轮定案）：标记为自动执行——whats-next 时不提问直接派发。
    let auto = args.iter().any(|a| a == "--auto");
    // 多个参数按空格拼接（`todo add fix the login bug` 免引号）。
    let text = args
        .iter()
        .filter(|a| *a != "--auto")
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let agent = crate::agents::detect::detect_invoking_agent();
    let added = match agent {
        Some(agent) => crate::todos::add_from_agent(project, &text, auto, agent),
        None if auto => crate::todos::add_auto(project, &text),
        None => crate::todos::add(project, &text),
    };
    let entry = match added {
        Ok(entry) => entry,
        Err(crate::todos::AddError::EmptyInput) => {
            eprintln!(
                "{}{}",
                i18n::err_prefix(lang),
                i18n::tr(lang, "todo.missingText")
            );
            exit(1);
        }
        Err(crate::todos::AddError::Persist) => {
            eprintln!(
                "{}{}",
                i18n::err_prefix(lang),
                i18n::tr(lang, "todo.persistFailed")
            );
            exit(1);
        }
    };
    // Prefer the real 1-based index of the new id (never report #0 on a hollow write).
    let n = crate::todos::index_of(project, &entry.id).unwrap_or(0);
    if n == 0 {
        eprintln!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "todo.persistFailed")
        );
        exit(1);
    }
    let key = if auto { "todo.addedAuto" } else { "todo.added" };
    print_line(
        &i18n::tr(lang, key)
            .replace("{n}", &n.to_string())
            .replace("{text}", text.trim()),
    );
    exit(0);
}

fn list(project: &str, lang: Lang) -> ! {
    let entries = crate::todos::list(project);
    if entries.is_empty() {
        print_line(i18n::tr(lang, "todo.empty"));
        exit(0);
    }
    print_line(
        &i18n::tr(lang, "todo.listHeader")
            .replace("{project}", &crate::project::display_name(project)),
    );
    for (i, entry) in entries.iter().enumerate() {
        let auto_mark = if entry.auto {
            format!(" {}", i18n::tr(lang, "todo.autoMark"))
        } else {
            String::new()
        };
        print_line(&format!("{:>3}. {}{}", i + 1, entry.text, auto_mark));
    }
    exit(0);
}

fn rm(project: &str, args: &[String], lang: Lang) -> ! {
    let raw = args.first().map(String::as_str).unwrap_or("");
    let entries = crate::todos::list(project);
    // 编号为 `todo list` 显示的 1 基序号。
    let index = raw
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=entries.len()).contains(n));
    let Some(index) = index else {
        eprintln!(
            "{}{}",
            i18n::err_prefix(lang),
            i18n::tr(lang, "todo.invalidIndex")
                .replace("{n}", raw)
                .replace("{prog}", &super::help::program_name())
        );
        exit(1);
    };
    let entry = &entries[index - 1];
    // 并发下条目可能已被别处删除（best-effort，与 D11 一致）：仍按成功报告，最终状态一致。
    crate::todos::remove(project, &entry.id);
    print_line(
        &i18n::tr(lang, "todo.removed")
            .replace("{n}", &index.to_string())
            .replace("{text}", &entry.text),
    );
    exit(0);
}

fn clear(project: &str, args: &[String], lang: Lang) -> ! {
    let entries = crate::todos::list(project);
    if entries.is_empty() {
        print_line(i18n::tr(lang, "todo.empty"));
        exit(0);
    }
    // 交互确认（D6：clear 需确认），`--yes`/`-y` 跳过（脚本用）。
    let skip_confirm = args.iter().any(|a| a == "--yes" || a == "-y");
    if !skip_confirm && !confirm_clear(entries.len(), lang) {
        print_line(i18n::tr(lang, "todo.clearAborted"));
        exit(0);
    }
    let removed = crate::todos::clear(project);
    print_line(&i18n::tr(lang, "todo.cleared").replace("{n}", &removed.to_string()));
    exit(0);
}

/// 读取一行确认（y/yes 才通过）；stdin 不可读 / EOF 视为取消。
fn confirm_clear(count: usize, lang: Lang) -> bool {
    use std::io::{BufRead, Write};
    let prompt = i18n::tr(lang, "todo.clearConfirm").replace("{n}", &count.to_string());
    let mut out = std::io::stdout();
    let _ = write!(out, "{prompt}").and_then(|_| out.flush());
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
