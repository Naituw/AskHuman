//! `AskHuman dev <enable|disable|status|preset …>` — Dev Instance management.
//!
//! See `docs/specs/dev-instance-parallel.md`.

use crate::config::AppConfig;
use crate::dev_instance::{self, DEV_DIR, ENABLED_MARKER};
use crate::dev_presets;
use crate::i18n::Lang;
use std::path::{Path, PathBuf};
use std::process::exit;

pub fn dispatch(args: &[String], _lang: Lang) {
    if args.is_empty() {
        print_usage();
        exit(1);
    }
    match args[0].as_str() {
        "migrate" => {
            let result = match args.get(1).map(String::as_str) {
                Some("plan") => crate::development::migration::plan().map(Some),
                Some("status") => crate::development::migration::latest(),
                Some("apply") => args
                    .get(2)
                    .ok_or_else(|| "missing migration ID".to_string())
                    .and_then(|id| crate::development::migration::apply(id))
                    .map(Some),
                Some("rollback") => args
                    .get(2)
                    .ok_or_else(|| "missing migration ID".to_string())
                    .and_then(|id| crate::development::migration::rollback(id))
                    .map(Some),
                _ => Err("use dev migrate plan|status|apply <id>|rollback <id>".into()),
            };
            match result {
                Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1)
                }
            }
            exit(0);
        }
        "migrate-worker" => {
            let result = match (args.get(1), args.get(2)) {
                (Some(agent), Some(mode)) => crate::development::migration::worker(agent, mode),
                _ => Err("missing migration worker action".into()),
            };
            if let Err(e) = result {
                eprintln!("error: {e}");
                exit(1)
            }
            exit(0);
        }
        "session" => {
            match crate::development::session::run_cli(&args[1..]) {
                Ok(receipt) => println!("{receipt}"),
                Err(error) => {
                    eprintln!("error: {error}");
                    exit(1);
                }
            }
            exit(0);
        }
        "session-launch" => {
            let result = match (args.get(1), args.get(2)) {
                (Some(id), Some(token)) => crate::development::session::launch_active(
                    id,
                    token,
                    args.get(3).is_some_and(|arg| arg == "--stdio"),
                ),
                _ => Err("session-launch requires a profile and launch token".into()),
            };
            match result {
                Ok(receipt) => println!("{receipt}"),
                Err(error) => {
                    eprintln!("error: {error}");
                    exit(1);
                }
            }
            exit(0);
        }
        "session-configure" => {
            let result = args
                .get(1)
                .ok_or_else(|| "missing development profile ID".to_string())
                .and_then(|id| crate::development::session::configure_active(id));
            if let Err(error) = result {
                eprintln!("error: {error}");
                exit(1);
            }
            exit(0);
        }
        "ask" => {
            if let Err(error) = crate::development::verification::run_cli(&args[1..]) {
                eprintln!("error: {error}");
                exit(1);
            }
            exit(0);
        }
        "target" => {
            let result = crate::development::routing::diagnostic().map(Some);
            match result {
                Ok(target) => println!("{}", serde_json::to_string_pretty(&target).unwrap()),
                Err(error) => {
                    eprintln!("error: {error}");
                    exit(1);
                }
            }
            exit(0);
        }
        "register-system" => {
            let Some(binary) = args.get(1) else {
                eprintln!("error: register-system requires a binary path");
                exit(1);
            };
            let result = std::env::current_dir()
                .map_err(|e| e.to_string())
                .map(|cwd| {
                    let path = PathBuf::from(binary);
                    if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    }
                })
                .and_then(|binary| {
                    let canonical = binary.canonicalize().map_err(|e| e.to_string())?;
                    if canonical
                        .components()
                        .any(|part| part.as_os_str() == crate::dev_instance::DEV_DIR)
                    {
                        return Err("system entry cannot point into a Dev Instance".into());
                    }
                    crate::development::routing::check_capability(&binary)?;
                    crate::development::state::update(|state| {
                        if state.version == 0 && args.iter().any(|arg| arg == "--new-install") {
                            state.mode = crate::development::state::RoutingMode::Explicit;
                        }
                        state.system_binary = Some(binary);
                        Ok(())
                    })
                    .map(|_| ())
                });
            if let Err(error) = result {
                eprintln!("error: {error}");
                exit(1);
            }
            exit(0);
        }
        "register-worktree" => {
            let Some(root) = args.get(1) else {
                eprintln!("error: register-worktree requires a path");
                exit(1);
            };
            if let Err(error) = crate::development::state::register_worktree(Path::new(root)) {
                eprintln!("error: {error}");
                exit(1);
            }
            exit(0);
        }
        "build-verify" => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(require_git_root);
            match crate::development::build::verify(&root) {
                Ok(info) => println!("{}", serde_json::to_string_pretty(&info).unwrap()),
                Err(error) => {
                    eprintln!("error: {error}");
                    exit(1);
                }
            }
            exit(0);
        }
        "enable" => cmd_enable(&args[1..]),
        "disable" => cmd_disable(&args[1..]),
        "status" => cmd_status(),
        "preset" => cmd_preset(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            exit(0);
        }
        other => {
            eprintln!("error: unknown dev subcommand '{other}'");
            print_usage();
            exit(1);
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:
  AskHuman dev enable [--preset <name>]... [--force]
  AskHuman dev disable [--purge]
  AskHuman dev status
  AskHuman dev ask [--worktree <path>] [--delivery local|remote|channels] -- <question>
  AskHuman dev ask --worktree <path> --resume <verification-id>
  AskHuman dev session <agent> [--worktree <path>] [--task <text>] [--prepare-only|--stdio]
  AskHuman --instance system dev migrate plan|status|apply <id>|rollback <id>
  AskHuman --instance system dev target
  AskHuman dev preset save <name> [--from-instance]
  AskHuman dev preset list
  AskHuman dev preset show <name>
  AskHuman dev preset release <name>
  AskHuman dev preset rm <name> [--force]

Dev Instance isolates daemon/bin/config per git worktree.
See docs/specs/dev-instance-parallel.md and docs/agent-worktree-setup.md."
    );
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    if cur.is_file() {
        cur.pop();
    }
    loop {
        let git = cur.join(".git");
        if git.exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn require_git_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match find_git_root(&cwd) {
        Some(r) => r,
        None => {
            eprintln!(
                "error: not inside a git worktree (no .git found walking up from {})",
                cwd.display()
            );
            exit(1);
        }
    }
}

fn seed_config_if_missing(home: &Path) -> Result<(), String> {
    let cfg_path = home.join("config.json");
    if cfg_path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    // Default AppConfig: popup on, all IM off — safe for isolated dev.
    let cfg = AppConfig::default();
    cfg.save_to(&cfg_path).map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_layout(root: &Path) -> Result<(PathBuf, PathBuf), String> {
    let dev = root.join(DEV_DIR);
    std::fs::create_dir_all(&dev).map_err(|e| e.to_string())?;
    let private = dev.canonicalize().map_err(|e| e.to_string())?;
    if !private.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Dev directory escapes the selected worktree".into());
    }
    let bin = dev.join("bin");
    let home = dev.join("home");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    if !bin
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&private)
        || !home
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&private)
    {
        return Err("Dev bin or home escapes the selected private worktree directory".into());
    }
    let marker = dev.join(ENABLED_MARKER);
    if !marker.exists() {
        std::fs::write(&marker, b"").map_err(|e| e.to_string())?;
    }
    seed_config_if_missing(&home)?;
    Ok((bin, home))
}

fn pin_instance_management(root: &Path) {
    if let Err(error) = crate::development::routing::pin_management(root) {
        eprintln!("error: {error}");
        exit(1);
    }
}

fn parse_enable_args(args: &[String]) -> (Vec<String>, bool) {
    let mut presets = Vec::new();
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--force" => force = true,
            "--preset" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --preset requires a name");
                    exit(1);
                }
                presets.push(args[i].clone());
            }
            other if other.starts_with("--preset=") => {
                presets.push(other.trim_start_matches("--preset=").to_string());
            }
            other => {
                eprintln!("error: unknown enable flag '{other}'");
                print_usage();
                exit(1);
            }
        }
        i += 1;
    }
    (presets, force)
}

fn cmd_enable(args: &[String]) {
    let (presets, force) = parse_enable_args(args);
    let root = require_git_root();
    let (bin, home) = match ensure_layout(&root) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    };

    // Pin env so subsequent logic in this process uses instance home (also for preset materialise).
    pin_instance_management(&root);

    if !presets.is_empty() {
        match dev_presets::apply_presets_to_instance(&root, &home, &presets, force) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("error: {e}");
                exit(1);
            }
        }
    }

    if let Err(error) = crate::development::state::register_worktree(&root) {
        eprintln!("error: cannot register Dev worktree: {error}");
        exit(1);
    }

    println!("Dev Instance enabled");
    println!("  worktree   {}", root.display());
    println!("  home       {}", home.display());
    println!("  bin        {}", bin.display());
    if presets.is_empty() {
        println!("  channels   popup-only (no IM presets)");
    } else {
        println!("  presets    {}", presets.join(", "));
    }
    if !bin
        .join(if cfg!(windows) {
            "AskHuman.exe"
        } else {
            "AskHuman"
        })
        .is_file()
    {
        println!("  next       run ./scripts/install.sh  (installs into this instance bin)");
    } else {
        if crate::development::state::read()
            .is_ok_and(|state| state.mode == crate::development::state::RoutingMode::Explicit)
        {
            println!("  next       AskHuman dev ask … / MCP dev_ask  (explicit Dev verification)");
        } else {
            println!(
                "  next       AskHuman … / MCP ask  (legacy automatic routing; see dev migrate)"
            );
        }
    }
    exit(0);
}

fn cmd_disable(args: &[String]) {
    let mut purge = false;
    for a in args {
        match a.as_str() {
            "--purge" => purge = true,
            other => {
                eprintln!("error: unknown disable flag '{other}'");
                exit(1);
            }
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(root) = dev_instance::find_dev_root(&cwd).or_else(|| {
        // Also allow disable from git root after marker deleted? Prefer find by git + .askhuman-dev
        find_git_root(&cwd).filter(|r| r.join(DEV_DIR).exists())
    }) else {
        eprintln!("error: no Dev Instance found from cwd {}", cwd.display());
        exit(1);
    };

    pin_instance_management(&root);
    if purge {
        if let Err(e) = crate::development::session::cleanup_workspaces(&root) {
            eprintln!("error: {e}");
            exit(1)
        }
    }

    // Best-effort stop the daemon in this instance partition before removing its marker/data.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    if let Ok(rt) = rt {
        rt.block_on(async {
            if crate::client::request_stop(true).await {
                crate::client::wait_until_down(std::time::Duration::from_secs(5)).await;
            }
            if crate::gui_host::shutdown_if_running().await {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while std::time::Instant::now() < deadline {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if !crate::gui_host::shutdown_if_running().await {
                        break;
                    }
                }
            }
        });
    }

    #[cfg(windows)]
    {
        let remaining = terminate_instance_processes(&dev_instance::instance_bin(&root));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while remaining
            .iter()
            .any(|pid| crate::agents::detect::pid_alive(*pid))
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    match dev_presets::release_leases_for_worktree(&root) {
        Ok(names) if !names.is_empty() => {
            println!("released preset leases: {}", names.join(", "));
        }
        Ok(_) => {}
        Err(e) => eprintln!("warning: failed to release preset leases: {e}"),
    }

    let marker = root.join(DEV_DIR).join(ENABLED_MARKER);
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
    }

    if purge {
        let dev = root.join(DEV_DIR);
        match remove_dev_dir(&dev) {
            Ok(()) => println!("purged {}", dev.display()),
            Err(e) => {
                eprintln!("error: failed to purge {}: {e}", dev.display());
                exit(1);
            }
        }
    } else {
        println!(
            "Dev Instance disabled (marker removed; bin/home kept at {})",
            root.join(DEV_DIR).display()
        );
        println!("  use --purge to delete bin/home as well");
    }
    exit(0);
}

fn remove_dev_dir(path: &Path) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if cfg!(windows) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(windows)]
fn terminate_instance_processes(instance_bin: &Path) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    fn normalized(path: &Path) -> String {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        canonical
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase()
    }

    let expected = normalized(instance_bin);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut terminated = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        let pid = entry.th32ProcessID;
        if pid != 0 && pid != std::process::id() {
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                    0,
                    pid,
                )
            };
            if !process.is_null() {
                let mut buffer = vec![0u16; 32_768];
                let mut length = buffer.len() as u32;
                let matches = unsafe {
                    QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length)
                } != 0
                    && normalized(Path::new(&String::from_utf16_lossy(
                        &buffer[..length as usize],
                    ))) == expected;
                if matches && unsafe { TerminateProcess(process, 0) } != 0 {
                    terminated.push(pid);
                }
                unsafe {
                    CloseHandle(process);
                }
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    terminated
}

fn cmd_status() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = dev_instance::find_dev_root(&cwd);
    match root {
        None => {
            println!("dev instance: not enabled for cwd {}", cwd.display());
            println!("  tip: AskHuman dev enable");
            exit(0);
        }
        Some(root) => {
            let home = dev_instance::instance_home(&root);
            let bin = dev_instance::instance_bin(&root);
            pin_instance_management(&root);

            println!("dev instance: enabled");
            println!("  worktree   {}", root.display());
            println!("  home       {}", home.display());
            println!(
                "  bin        {} ({})",
                bin.display(),
                if bin.is_file() {
                    "present"
                } else {
                    "missing — run ./scripts/install.sh"
                }
            );

            let meta = dev_presets::read_meta(&home);
            if meta.applied_presets.is_empty() {
                println!("  presets    (none)");
            } else {
                println!("  presets    {}", meta.applied_presets.join(", "));
            }

            let cfg = AppConfig::load_from(&home.join("config.json"));
            let ids = dev_presets::configured_channel_ids(&cfg.channels);
            if ids.is_empty() {
                println!("  channels   popup-only");
            } else {
                println!("  channels   {}", ids.join(", "));
            }

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(rt) = rt {
                match rt.block_on(crate::client::request_status()) {
                    Some(st) => {
                        println!(
                            "  daemon     running pid={} version={} requests={}",
                            st.pid, st.version, st.active_requests
                        );
                    }
                    None => println!("  daemon     not running"),
                }
            } else {
                println!("  daemon     status unavailable");
            }
            exit(0);
        }
    }
}

fn cmd_preset(args: &[String]) {
    if args.is_empty() {
        eprintln!("error: missing preset subcommand");
        print_usage();
        exit(1);
    }
    match args[0].as_str() {
        "save" => preset_save(&args[1..]),
        "list" => preset_list(),
        "show" => {
            if args.len() < 2 {
                eprintln!("error: preset show requires <name>");
                exit(1);
            }
            preset_show(&args[1]);
        }
        "release" => {
            if args.len() < 2 {
                eprintln!("error: preset release requires <name>");
                exit(1);
            }
            match dev_presets::release_lease(&args[1]) {
                Ok(()) => {
                    println!("released lease for preset '{}'", args[1]);
                    exit(0);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        "rm" | "remove" => {
            let mut force = false;
            let mut name = None;
            for a in &args[1..] {
                if a == "--force" {
                    force = true;
                } else if name.is_none() {
                    name = Some(a.as_str());
                } else {
                    eprintln!("error: unexpected argument '{a}'");
                    exit(1);
                }
            }
            let Some(name) = name else {
                eprintln!("error: preset rm requires <name>");
                exit(1);
            };
            match dev_presets::remove_preset(name, force) {
                Ok(()) => {
                    println!("removed preset '{name}'");
                    exit(0);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("error: unknown preset subcommand '{other}'");
            print_usage();
            exit(1);
        }
    }
}

fn preset_save(args: &[String]) {
    let mut from_instance = false;
    let mut name = None;
    for a in args {
        match a.as_str() {
            "--from-instance" => from_instance = true,
            other if !other.starts_with('-') && name.is_none() => name = Some(other.to_string()),
            other => {
                eprintln!("error: unknown save flag/arg '{other}'");
                exit(1);
            }
        }
    }
    let Some(name) = name else {
        eprintln!("error: preset save requires <name>");
        exit(1);
    };
    if !from_instance {
        eprintln!(
            "error: only --from-instance is supported for now (configure channels in this worktree first)"
        );
        eprintln!("  AskHuman dev enable && AskHuman --settings && AskHuman dev preset save {name} --from-instance");
        exit(1);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(root) = dev_instance::find_dev_root(&cwd) else {
        eprintln!("error: cwd is not a Dev Instance (run `dev enable` first)");
        exit(1);
    };
    pin_instance_management(&root);

    let cfg = AppConfig::load();
    let channels = match dev_presets::extract_configured_channels(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    };
    let ids = dev_presets::configured_channel_ids(&channels);
    match dev_presets::save_preset(&name, channels) {
        Ok(()) => {
            println!("saved preset '{name}' ({})", ids.join(", "));
            exit(0);
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    }
}

fn preset_list() {
    let list = dev_presets::list_presets();
    if list.is_empty() {
        println!("(no presets)");
        exit(0);
    }
    for (name, lease, channels) in list {
        let ch = if channels.is_empty() {
            "empty".to_string()
        } else {
            channels.join(",")
        };
        match lease {
            Some(l) => println!("{name}  channels={ch}  leased_by={}", l.worktree_root),
            None => println!("{name}  channels={ch}  lease=free"),
        }
    }
    exit(0);
}

fn preset_show(name: &str) {
    match dev_presets::show_preset(name) {
        Ok((body, lease)) => {
            println!("preset: {name}");
            match lease {
                Some(l) => println!("lease:  {}", l.worktree_root),
                None => println!("lease:  free"),
            }
            let redacted = dev_presets::redact_channels(&body.channels);
            println!(
                "{}",
                serde_json::to_string_pretty(&redacted).unwrap_or_default()
            );
            exit(0);
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    }
}
