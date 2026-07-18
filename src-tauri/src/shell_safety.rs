//! Replicated Codex shell-command safety logic (spec `docs/specs/codex-permission-remember.md`,
//! D31/D32/D34).
//!
//! This module is a line-faithful port of the Unix-relevant parts of Codex
//! `codex-rs/shell-command` (`bash.rs`, `command_safety/is_safe_command.rs`,
//! `command_safety/is_dangerous_command.rs`) plus the `BANNED_PREFIX_SUGGESTIONS` table from
//! `core/src/exec_policy.rs`. It must only ever be used behind the version gate
//! ([`codex_version_supported`], D35): the port was verified against the source tree of the
//! version ceiling below, and drift in newer Codex releases must fail closed to the basic
//! popup rather than run stale heuristics.
//!
//! Windows-only branches of the original (PowerShell safelists, .exe name stripping) are
//! intentionally omitted: the AskHuman permission hook only runs on Unix.

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

/// Inclusive `(major, minor)` window of Codex versions this port was verified against
/// (D35). Outside the window, shell memory options are disabled (fail closed to basic).
pub const VERIFIED_CODEX_VERSION_FLOOR: (u32, u32) = (0, 144);
pub const VERIFIED_CODEX_VERSION_CEILING: (u32, u32) = (0, 144);

/// Parses `codex-cli 0.144.4` (the `codex --version` output) into `(major, minor)`.
pub fn parse_codex_version(output: &str) -> Option<(u32, u32)> {
    let version = output.trim().rsplit(' ').next()?;
    let mut parts = version.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

pub fn codex_version_supported(version: (u32, u32)) -> bool {
    version >= VERIFIED_CODEX_VERSION_FLOOR && version <= VERIFIED_CODEX_VERSION_CEILING
}

/// Prefix rules Codex refuses to suggest or accept as amendments
/// (`BANNED_PREFIX_SUGGESTIONS` in `core/src/exec_policy.rs`).
pub static BANNED_PREFIX_SUGGESTIONS: &[&[&str]] = &[
    &["python3"],
    &["python3", "-"],
    &["python3", "-c"],
    &["python"],
    &["python", "-"],
    &["python", "-c"],
    &["py"],
    &["py", "-3"],
    &["pythonw"],
    &["pyw"],
    &["pypy"],
    &["pypy3"],
    &["git"],
    &["bash"],
    &["bash", "-lc"],
    &["sh"],
    &["sh", "-c"],
    &["sh", "-lc"],
    &["zsh"],
    &["zsh", "-lc"],
    &["/bin/zsh"],
    &["/bin/zsh", "-lc"],
    &["/bin/bash"],
    &["/bin/bash", "-lc"],
    &["pwsh"],
    &["pwsh", "-Command"],
    &["pwsh", "-c"],
    &["powershell"],
    &["powershell", "-Command"],
    &["powershell", "-c"],
    &["powershell.exe"],
    &["powershell.exe", "-Command"],
    &["powershell.exe", "-c"],
    &["env"],
    &["sudo"],
    &["node"],
    &["node", "-e"],
    &["perl"],
    &["perl", "-e"],
    &["ruby"],
    &["ruby", "-e"],
    &["php"],
    &["php", "-r"],
    &["lua"],
    &["lua", "-e"],
    &["osascript"],
];

pub fn is_banned_prefix(prefix: &[String]) -> bool {
    BANNED_PREFIX_SUGGESTIONS.iter().any(|banned| {
        prefix.len() == banned.len() && prefix.iter().map(String::as_str).eq(banned.iter().copied())
    })
}

// ===== bash.rs port =====

fn try_parse_shell(shell_lc_arg: &str) -> Option<Tree> {
    let lang = tree_sitter_bash::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    parser.parse(shell_lc_arg, None)
}

fn try_parse_word_only_commands_sequence(tree: &Tree, src: &str) -> Option<Vec<Vec<String>>> {
    if tree.root_node().has_error() {
        return None;
    }

    const ALLOWED_KINDS: &[&str] = &[
        "program",
        "list",
        "pipeline",
        "command",
        "command_name",
        "word",
        "string",
        "string_content",
        "raw_string",
        "number",
        "concatenation",
    ];
    const ALLOWED_PUNCT_TOKENS: &[&str] = &["&&", "||", ";", "|", "\"", "'"];

    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut stack = vec![root];
    let mut command_nodes = Vec::new();
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if node.is_named() {
            if !ALLOWED_KINDS.contains(&kind) {
                return None;
            }
            if kind == "command" {
                command_nodes.push(node);
            }
        } else {
            if kind.chars().any(|c| "&;|".contains(c)) && !ALLOWED_PUNCT_TOKENS.contains(&kind) {
                return None;
            }
            if !(ALLOWED_PUNCT_TOKENS.contains(&kind) || kind.trim().is_empty()) {
                return None;
            }
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    command_nodes.sort_by_key(Node::start_byte);

    let mut commands = Vec::new();
    for node in command_nodes {
        commands.push(parse_plain_command_from_node(node, src)?);
    }
    Some(commands)
}

/// Parses a shell script consisting only of plain commands joined by `&&`, `||`, `;`, `|`.
pub fn parse_shell_script_into_commands(script: &str) -> Option<Vec<Vec<String>>> {
    let tree = try_parse_shell(script)?;
    try_parse_word_only_commands_sequence(&tree, script)
}

/// Unix subset of Codex `detect_shell_type`: basename must be bash / zsh / sh.
fn is_supported_shell(shell: &str) -> bool {
    let basename = Path::new(shell)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(shell);
    matches!(basename, "bash" | "zsh" | "sh")
}

fn extract_bash_command(command: &[String]) -> Option<(&str, &str)> {
    let [shell, flag, script] = command else {
        return None;
    };
    if !matches!(flag.as_str(), "-lc" | "-c") || !is_supported_shell(shell) {
        return None;
    }
    Some((shell, script))
}

/// `bash -lc "..."` split into plain word-only commands, exactly like
/// `parse_shell_lc_plain_commands`.
pub fn parse_shell_lc_plain_commands(command: &[String]) -> Option<Vec<Vec<String>>> {
    let (_, script) = extract_bash_command(command)?;
    parse_shell_script_into_commands(script)
}

/// Literal portions of every command node in a complex script (dangerous-command probing
/// only; must never be used to prove safety).
fn parse_shell_lc_literal_commands(command: &[String]) -> Option<Vec<Vec<String>>> {
    let (_, script) = extract_bash_command(command)?;
    let tree = try_parse_shell(script)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut commands = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "command" {
            if let Some(command) = parse_literal_command_from_node(node, script) {
                commands.push(command);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }

    Some(commands)
}

fn parse_plain_command_from_node(cmd: Node<'_>, src: &str) -> Option<Vec<String>> {
    if cmd.kind() != "command" {
        return None;
    }
    let mut words = Vec::new();
    let mut cursor = cmd.walk();
    for child in cmd.named_children(&mut cursor) {
        match child.kind() {
            "command_name" => {
                let word_node = child.named_child(0)?;
                if word_node.kind() != "word" {
                    return None;
                }
                words.push(word_node.utf8_text(src.as_bytes()).ok()?.to_owned());
            }
            "word" | "number" => {
                words.push(child.utf8_text(src.as_bytes()).ok()?.to_owned());
            }
            "string" => words.push(parse_double_quoted_string(child, src)?),
            "raw_string" => words.push(parse_raw_string(child, src)?),
            "concatenation" => {
                let mut concatenated = String::new();
                let mut concat_cursor = child.walk();
                for part in child.named_children(&mut concat_cursor) {
                    match part.kind() {
                        "word" | "number" => {
                            concatenated.push_str(part.utf8_text(src.as_bytes()).ok()?);
                        }
                        "string" => concatenated.push_str(&parse_double_quoted_string(part, src)?),
                        "raw_string" => concatenated.push_str(&parse_raw_string(part, src)?),
                        _ => return None,
                    }
                }
                if concatenated.is_empty() {
                    return None;
                }
                words.push(concatenated);
            }
            _ => return None,
        }
    }
    Some(words)
}

fn parse_literal_command_from_node(cmd: Node<'_>, src: &str) -> Option<Vec<String>> {
    if cmd.kind() != "command" {
        return None;
    }

    let mut words = Vec::new();
    let mut found_command_name = false;
    let mut cursor = cmd.walk();
    for child in cmd.named_children(&mut cursor) {
        if child.kind() == "command_name" {
            let command_name = parse_literal_shell_word(child.named_child(0)?, src)?;
            words.push(command_name);
            found_command_name = true;
        } else if found_command_name {
            if let Some(word) = parse_literal_shell_word(child, src) {
                words.push(word);
            }
        }
    }

    found_command_name.then_some(words)
}

fn parse_literal_shell_word(node: Node<'_>, src: &str) -> Option<String> {
    match node.kind() {
        "word" | "number" if is_literal_word_or_number(node) => {
            Some(node.utf8_text(src.as_bytes()).ok()?.to_owned())
        }
        "string" => parse_double_quoted_string(node, src),
        "raw_string" => parse_raw_string(node, src),
        "concatenation" => {
            let mut concatenated = String::new();
            let mut cursor = node.walk();
            for part in node.named_children(&mut cursor) {
                concatenated.push_str(&parse_literal_shell_word(part, src)?);
            }
            (!concatenated.is_empty()).then_some(concatenated)
        }
        _ => None,
    }
}

fn is_literal_word_or_number(node: Node<'_>) -> bool {
    if !matches!(node.kind(), "word" | "number") {
        return false;
    }
    let mut cursor = node.walk();
    let has_children = node.named_children(&mut cursor).next().is_some();
    !has_children
}

fn parse_double_quoted_string(node: Node<'_>, src: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    for part in node.named_children(&mut cursor) {
        if part.kind() != "string_content" {
            return None;
        }
    }
    let raw = node.utf8_text(src.as_bytes()).ok()?;
    let stripped = raw
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))?;
    Some(stripped.to_string())
}

fn parse_raw_string(node: Node<'_>, src: &str) -> Option<String> {
    if node.kind() != "raw_string" {
        return None;
    }
    let raw_string = node.utf8_text(src.as_bytes()).ok()?;
    raw_string
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .map(str::to_owned)
}

// ===== is_safe_command.rs port (Unix branches) =====

pub fn is_known_safe_command(command: &[String]) -> bool {
    let command: Vec<String> = command
        .iter()
        .map(|s| {
            if s == "zsh" {
                "bash".to_string()
            } else {
                s.clone()
            }
        })
        .collect();

    if is_safe_to_call_with_exec(&command) {
        return true;
    }

    if let Some(all_commands) = parse_shell_lc_plain_commands(&command) {
        if !all_commands.is_empty()
            && all_commands
                .iter()
                .all(|cmd| is_safe_to_call_with_exec(cmd))
        {
            return true;
        }
    }
    false
}

fn is_safe_to_call_with_exec(command: &[String]) -> bool {
    let Some(cmd0) = command.first().map(String::as_str) else {
        return false;
    };

    match executable_name_lookup_key(cmd0).as_deref() {
        Some(cmd) if cfg!(target_os = "linux") && matches!(cmd, "numfmt" | "tac") => true,

        Some(
            "cat" | "cd" | "cut" | "echo" | "expr" | "false" | "grep" | "head" | "id" | "ls" | "nl"
            | "paste" | "pwd" | "rev" | "seq" | "stat" | "tail" | "tr" | "true" | "uname" | "uniq"
            | "wc" | "which" | "whoami",
        ) => true,

        Some("base64") => {
            const UNSAFE_BASE64_OPTIONS: &[&str] = &["-o", "--output"];
            !command.iter().skip(1).any(|arg| {
                UNSAFE_BASE64_OPTIONS.contains(&arg.as_str())
                    || arg.starts_with("--output=")
                    || (arg.starts_with("-o") && arg != "-o")
            })
        }

        Some("find") => {
            const UNSAFE_FIND_OPTIONS: &[&str] = &[
                "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fls", "-fprint", "-fprint0",
                "-fprintf",
            ];
            !command
                .iter()
                .any(|arg| UNSAFE_FIND_OPTIONS.contains(&arg.as_str()))
        }

        Some("rg") => {
            const UNSAFE_RIPGREP_OPTIONS_WITH_ARGS: &[&str] = &["--pre", "--hostname-bin"];
            const UNSAFE_RIPGREP_OPTIONS_WITHOUT_ARGS: &[&str] = &["--search-zip", "-z"];
            !command.iter().any(|arg| {
                UNSAFE_RIPGREP_OPTIONS_WITHOUT_ARGS.contains(&arg.as_str())
                    || UNSAFE_RIPGREP_OPTIONS_WITH_ARGS
                        .iter()
                        .any(|&opt| arg == opt || arg.starts_with(&format!("{opt}=")))
            })
        }

        Some("git") => is_safe_git_command(command),

        Some("sed")
            if {
                command.len() <= 4
                    && command.get(1).map(String::as_str) == Some("-n")
                    && is_valid_sed_n_arg(command.get(2).map(String::as_str))
            } =>
        {
            true
        }

        _ => false,
    }
}

fn is_safe_git_command(command: &[String]) -> bool {
    let Some((subcommand_idx, subcommand)) =
        find_git_subcommand(command, &["status", "log", "diff", "show", "branch"])
    else {
        return false;
    };

    let global_args = &command[1..subcommand_idx];
    if git_has_unsafe_global_option(global_args) {
        return false;
    }

    let subcommand_args = &command[subcommand_idx + 1..];

    match subcommand {
        "status" | "log" | "diff" | "show" => git_subcommand_args_are_read_only(subcommand_args),
        "branch" => {
            git_subcommand_args_are_read_only(subcommand_args)
                && git_branch_is_read_only(subcommand_args)
        }
        _ => false,
    }
}

fn git_branch_is_read_only(branch_args: &[String]) -> bool {
    if branch_args.is_empty() {
        return true;
    }

    let mut saw_read_only_flag = false;
    for arg in branch_args.iter().map(String::as_str) {
        match arg {
            "--list" | "-l" | "--show-current" | "-a" | "--all" | "-r" | "--remotes" | "-v"
            | "-vv" | "--verbose" => {
                saw_read_only_flag = true;
            }
            _ if arg.starts_with("--format=") => {
                saw_read_only_flag = true;
            }
            _ => return false,
        }
    }

    saw_read_only_flag
}

#[derive(Clone, Copy)]
enum GitOptionPattern {
    Exact(&'static str),
    ShortWithInlineValue(&'static str),
    Prefix(&'static str),
}

const UNSAFE_GIT_GLOBAL_OPTIONS: &[GitOptionPattern] = &[
    GitOptionPattern::Exact("-C"),
    GitOptionPattern::ShortWithInlineValue("-C"),
    GitOptionPattern::Exact("-c"),
    GitOptionPattern::ShortWithInlineValue("-c"),
    GitOptionPattern::Exact("-p"),
    GitOptionPattern::Exact("--config-env"),
    GitOptionPattern::Prefix("--config-env="),
    GitOptionPattern::Exact("--exec-path"),
    GitOptionPattern::Prefix("--exec-path="),
    GitOptionPattern::Exact("--git-dir"),
    GitOptionPattern::Prefix("--git-dir="),
    GitOptionPattern::Exact("--namespace"),
    GitOptionPattern::Prefix("--namespace="),
    GitOptionPattern::Exact("--paginate"),
    GitOptionPattern::Exact("--super-prefix"),
    GitOptionPattern::Prefix("--super-prefix="),
    GitOptionPattern::Exact("--work-tree"),
    GitOptionPattern::Prefix("--work-tree="),
];

const UNSAFE_GIT_SUBCOMMAND_OPTIONS: &[GitOptionPattern] = &[
    GitOptionPattern::Exact("--output"),
    GitOptionPattern::Prefix("--output="),
    GitOptionPattern::Exact("--ext-diff"),
    GitOptionPattern::Exact("--textconv"),
    GitOptionPattern::Exact("--exec"),
    GitOptionPattern::Prefix("--exec="),
];

impl GitOptionPattern {
    fn matches(self, arg: &str) -> bool {
        match self {
            GitOptionPattern::Exact(option) => arg == option,
            GitOptionPattern::ShortWithInlineValue(option) => {
                arg.starts_with(option) && arg.len() > option.len()
            }
            GitOptionPattern::Prefix(prefix) => arg.starts_with(prefix),
        }
    }
}

fn git_matches_option_pattern(arg: &str, patterns: &[GitOptionPattern]) -> bool {
    patterns.iter().any(|pattern| pattern.matches(arg))
}

fn git_has_unsafe_global_option(global_args: &[String]) -> bool {
    global_args
        .iter()
        .map(String::as_str)
        .any(|arg| git_matches_option_pattern(arg, UNSAFE_GIT_GLOBAL_OPTIONS))
}

fn git_subcommand_args_are_read_only(args: &[String]) -> bool {
    !args
        .iter()
        .map(String::as_str)
        .any(|arg| git_matches_option_pattern(arg, UNSAFE_GIT_SUBCOMMAND_OPTIONS))
}

/// Returns true if `arg` matches /^(\d+,)?\d+p$/
fn is_valid_sed_n_arg(arg: Option<&str>) -> bool {
    let s = match arg {
        Some(s) => s,
        None => return false,
    };
    let core = match s.strip_suffix('p') {
        Some(rest) => rest,
        None => return false,
    };
    let parts: Vec<&str> = core.split(',').collect();
    match parts.as_slice() {
        [num] => !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()),
        [a, b] => {
            !a.is_empty()
                && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

// ===== is_dangerous_command.rs port (Unix branches) =====

const MAX_DANGEROUS_COMMAND_WRAPPER_DEPTH: usize = 8;

/// Whether an already-tokenized command matches a dangerous-command rule.
pub fn is_dangerous_command(command: &[String]) -> bool {
    dangerous_command_match_with_depth(command, 0)
}

fn dangerous_command_match_with_depth(command: &[String], wrapper_depth: usize) -> bool {
    if wrapper_depth > MAX_DANGEROUS_COMMAND_WRAPPER_DEPTH {
        return false;
    }

    if dangerous_command_match_for_exec(command, wrapper_depth) {
        return true;
    }

    parse_shell_lc_literal_commands(command).is_some_and(|commands| {
        commands
            .iter()
            .any(|command| dangerous_command_match_with_depth(command, wrapper_depth + 1))
    })
}

fn executable_name_lookup_key(raw: &str) -> Option<String> {
    Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .map(std::borrow::ToOwned::to_owned)
}

fn find_git_subcommand<'a>(
    command: &'a [String],
    subcommands: &[&str],
) -> Option<(usize, &'a str)> {
    let cmd0 = command.first().map(String::as_str)?;
    if executable_name_lookup_key(cmd0).as_deref() != Some("git") {
        return None;
    }

    let mut skip_next = false;
    for (idx, arg) in command.iter().enumerate().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }

        let arg = arg.as_str();

        if is_git_global_option_with_inline_value(arg) {
            continue;
        }

        if is_git_global_option_with_value(arg) {
            skip_next = true;
            continue;
        }

        if arg == "--" || arg.starts_with('-') {
            continue;
        }

        if subcommands.contains(&arg) {
            return Some((idx, arg));
        }

        return None;
    }

    None
}

fn is_git_global_option_with_value(arg: &str) -> bool {
    matches!(
        arg,
        "-C" | "-c"
            | "--config-env"
            | "--exec-path"
            | "--git-dir"
            | "--namespace"
            | "--super-prefix"
            | "--work-tree"
    )
}

fn is_git_global_option_with_inline_value(arg: &str) -> bool {
    matches!(
        arg,
        s if s.starts_with("--config-env=")
            || s.starts_with("--exec-path=")
            || s.starts_with("--git-dir=")
            || s.starts_with("--namespace=")
            || s.starts_with("--super-prefix=")
            || s.starts_with("--work-tree=")
    ) || ((arg.starts_with("-C") || arg.starts_with("-c")) && arg.len() > 2)
}

fn dangerous_command_match_for_exec(command: &[String], wrapper_depth: usize) -> bool {
    let cmd0 = command
        .first()
        .and_then(|command| executable_name_lookup_key(command));

    match cmd0.as_deref() {
        Some("rm") if rm_args_include_force_option(&command[1..]) => true,
        Some("sudo") => dangerous_command_match_with_depth(&command[1..], wrapper_depth + 1),
        Some("env") => dangerous_command_match_for_env(command, wrapper_depth),
        Some("trap") => dangerous_command_match_for_trap(command, wrapper_depth),
        _ => false,
    }
}

fn dangerous_command_match_for_env(command: &[String], wrapper_depth: usize) -> bool {
    let mut command_index = 1;
    while let Some(argument) = command.get(command_index) {
        if argument == "--" {
            command_index += 1;
            break;
        }
        if matches!(argument.as_str(), "-i" | "--ignore-environment")
            || argument
                .split_once('=')
                .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'))
        {
            command_index += 1;
            continue;
        }
        break;
    }
    dangerous_command_match_with_depth(&command[command_index..], wrapper_depth + 1)
}

fn dangerous_command_match_for_trap(command: &[String], wrapper_depth: usize) -> bool {
    let mut action_index = 1;
    if command
        .get(action_index)
        .is_some_and(|argument| argument == "--")
    {
        action_index += 1;
    }
    let Some(action) = command
        .get(action_index)
        .filter(|action| !action.starts_with('-'))
    else {
        return false;
    };
    let shell_command = vec!["sh".to_string(), "-c".to_string(), action.clone()];
    dangerous_command_match_with_depth(&shell_command, wrapper_depth + 1)
}

fn rm_args_include_force_option(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            arg == "--force"
                || arg
                    .strip_prefix('-')
                    .is_some_and(|flags| !flags.starts_with('-') && flags.contains('f'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_str(args: &[&str]) -> Vec<String> {
        args.iter().map(std::string::ToString::to_string).collect()
    }

    // ===== parser (bash.rs upstream test vectors) =====

    fn parse_seq(src: &str) -> Option<Vec<Vec<String>>> {
        parse_shell_script_into_commands(src)
    }

    #[test]
    fn parser_matches_upstream_vectors() {
        assert_eq!(
            parse_seq("ls -1").unwrap(),
            vec![vec!["ls".to_string(), "-1".to_string()]]
        );
        assert_eq!(
            parse_seq("ls && pwd; echo 'hi there' | wc -l").unwrap(),
            vec![
                vec!["ls".to_string()],
                vec!["pwd".to_string()],
                vec!["echo".to_string(), "hi there".to_string()],
                vec!["wc".to_string(), "-l".to_string()],
            ]
        );
        assert_eq!(
            parse_seq("git commit -m \"line1\nline2\"").unwrap(),
            vec![vec![
                "git".to_string(),
                "commit".to_string(),
                "-m".to_string(),
                "line1\nline2".to_string(),
            ]]
        );
        assert_eq!(
            parse_seq(r#"echo "/usr"'/'"local"/bin"#).unwrap(),
            vec![vec!["echo".to_string(), "/usr/local/bin".to_string()]]
        );
        assert_eq!(
            parse_seq("rg -n \"foo\" -g\"*.py\"").unwrap(),
            vec![vec![
                "rg".to_string(),
                "-n".to_string(),
                "foo".to_string(),
                "-g*.py".to_string(),
            ]]
        );
    }

    #[test]
    fn parser_rejects_unsafe_constructs() {
        for src in [
            r#"echo "hi ${USER}""#,
            "(ls)",
            "ls || (pwd && echo hi)",
            "ls > out.txt",
            "echo hi & echo bye",
            "echo $(pwd)",
            "echo `pwd`",
            "echo $HOME",
            "FOO=bar ls",
            "ls &&",
            "&& ls",
            "ls ;; pwd",
            "ls | | wc",
            "rg -g\"$VAR\" pattern",
            "rg -g\"$(pwd)\" pattern",
        ] {
            assert!(parse_seq(src).is_none(), "should reject: {src}");
        }
    }

    #[test]
    fn shell_lc_wrapper_extraction() {
        assert_eq!(
            parse_shell_lc_plain_commands(&vec_str(&["zsh", "-lc", "ls"])).unwrap(),
            vec![vec!["ls".to_string()]]
        );
        assert_eq!(
            parse_shell_lc_plain_commands(&vec_str(&["/bin/bash", "-c", "pwd"])).unwrap(),
            vec![vec!["pwd".to_string()]]
        );
        assert!(parse_shell_lc_plain_commands(&vec_str(&["fish", "-lc", "ls"])).is_none());
        assert!(parse_shell_lc_plain_commands(&vec_str(&["bash", "-x", "ls"])).is_none());
        assert!(parse_shell_lc_plain_commands(&vec_str(&["bash", "-lc"])).is_none());
    }

    // ===== safe-command heuristics (upstream vectors) =====

    #[test]
    fn known_safe_examples() {
        assert!(is_known_safe_command(&vec_str(&["ls"])));
        assert!(is_known_safe_command(&vec_str(&["git", "status"])));
        assert!(is_known_safe_command(&vec_str(&[
            "git",
            "branch",
            "--show-current"
        ])));
        assert!(is_known_safe_command(&vec_str(&[
            "sed", "-n", "1,5p", "file.txt"
        ])));
        assert!(is_known_safe_command(&vec_str(&[
            "find", ".", "-name", "file.txt"
        ])));
        assert!(is_known_safe_command(&vec_str(&[
            "bash",
            "-lc",
            "ls && pwd"
        ])));
        assert!(is_known_safe_command(&vec_str(&[
            "bash",
            "-lc",
            "ls | wc -l"
        ])));
        assert!(is_known_safe_command(&vec_str(&["zsh", "-lc", "ls"])));
        assert!(is_known_safe_command(&vec_str(&["git", "log", "-p", "-1"])));
    }

    #[test]
    fn known_unsafe_examples() {
        assert!(!is_known_safe_command(&vec_str(&["cargo", "check"])));
        assert!(!is_known_safe_command(&vec_str(&["git", "fetch"])));
        assert!(!is_known_safe_command(&vec_str(&[
            "git", "branch", "-d", "feature"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "git", "-C", ".", "status"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "git",
            "--paginate",
            "log",
            "-1"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "git",
            "log",
            "--output=/tmp/x",
            "-n",
            "1"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "find", ".", "-name", "x", "-delete"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "rg", "--pre", "pwned", "f"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "base64", "-o", "out.bin"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "bash",
            "-lc",
            "ls && rm -rf /"
        ])));
        assert!(!is_known_safe_command(&vec_str(&["bash", "-lc", "(ls)"])));
        assert!(!is_known_safe_command(&vec_str(&[
            "bash",
            "-lc",
            "ls > out.txt"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "bash", "-lc", "git", "status"
        ])));
        assert!(!is_known_safe_command(&vec_str(&[
            "sed", "-n", "xp", "file.txt"
        ])));
    }

    // ===== dangerous-command heuristics (upstream vectors) =====

    #[test]
    fn dangerous_rm_variants() {
        for command in [
            vec_str(&["rm", "-rf", "/"]),
            vec_str(&["/bin/rm", "-fr", "/tmp/example"]),
            vec_str(&["rm", "-r", "-f", "/tmp/example"]),
            vec_str(&["rm", "--force", "/tmp/example"]),
            vec_str(&["rm", "/tmp/example", "-f"]),
            vec_str(&["sudo", "rm", "-rf", "/tmp/example"]),
            vec_str(&["env", "TARGET=/tmp/example", "rm", "-rf", "/tmp/example"]),
        ] {
            assert!(is_dangerous_command(&command), "{command:?}");
        }
    }

    #[test]
    fn dangerous_in_complex_shell_syntax() {
        for script in [
            "printf x | rm -rf /tmp/example",
            "if test -d /tmp/example; then rm --force /tmp/example; fi",
            "rm -rf \"$TARGET\" >/dev/null",
            "for target in /tmp/a /tmp/b; do rm -r -f \"$target\"; done",
            "echo \"$(rm -rf /tmp/example)\"",
            "bash -c 'rm -rf /tmp/example'",
            "trap 'rm -rf /tmp/example' EXIT",
        ] {
            let command = vec_str(&["bash", "-lc", script]);
            assert!(is_dangerous_command(&command), "{script}");
        }
    }

    #[test]
    fn non_dangerous_examples() {
        for command in [
            vec_str(&["rm", "-r", "/tmp/example"]),
            vec_str(&["rm", "--", "-f"]),
            vec_str(&["bash", "-lc", "echo 'rm -rf /tmp/example'"]),
            vec_str(&["bash", "-lc", "cmd=rm; $cmd -rf /tmp/example"]),
            vec_str(&["env", "TARGET=/tmp/example", "rm", "-r", "/tmp/example"]),
            vec_str(&["bash", "-lc", "trap 'echo rm -rf /tmp/example' EXIT"]),
        ] {
            assert!(!is_dangerous_command(&command), "{command:?}");
        }
    }

    // ===== version gate & banned prefixes =====

    #[test]
    fn version_gate_and_banned_prefixes() {
        assert_eq!(parse_codex_version("codex-cli 0.144.4"), Some((0, 144)));
        assert_eq!(parse_codex_version("codex-cli 1.2.3\n"), Some((1, 2)));
        assert_eq!(parse_codex_version("garbage"), None);
        assert!(codex_version_supported((0, 144)));
        assert!(!codex_version_supported((0, 143)));
        assert!(!codex_version_supported((0, 145)));
        assert!(!codex_version_supported((1, 0)));

        assert!(is_banned_prefix(&vec_str(&["git"])));
        assert!(is_banned_prefix(&vec_str(&["bash", "-lc"])));
        assert!(is_banned_prefix(&vec_str(&["sudo"])));
        assert!(!is_banned_prefix(&vec_str(&["git", "push"])));
        assert!(!is_banned_prefix(&vec_str(&["cargo"])));
    }
}
