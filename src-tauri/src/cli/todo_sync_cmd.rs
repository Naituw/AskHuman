//! Hidden `AskHuman todo-sync <status|tick|enable|disable|…>` (spec todo-sync §8): troubleshooting and
//! the entry point the sync helper starts for every trigger. Not part of the agent-facing help.

use super::print_line;
use crate::i18n::{self, Lang};
use crate::todo_sync::service::{self, ControlError};
use crate::todo_sync::tick::TickReport;
use std::process::exit;

pub fn dispatch(args: &[String], lang: Lang) -> ! {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("status") | None => status(rest),
        Some("tick") => tick(rest, lang),
        Some("enable") => control(service::enable(), "enabled", lang),
        Some("disable") => control(service::disable(), "disabled", lang),
        Some("resume") => match service::resume() {
            Ok(lifted) => {
                print_line(if lifted {
                    "resumed"
                } else {
                    "nothing to resume"
                });
                exit(0);
            }
            Err(e) => fail(lang, &e.to_string()),
        },
        Some("restart-from-local") => control(
            service::restart_from_local(),
            "this Mac's copy will be uploaded again",
            lang,
        ),
        Some("clear-cloud") => {
            // Deleting the cloud zone affects every Mac: never by accident.
            if !rest.iter().any(|a| a == "--yes") {
                fail(
                    lang,
                    "this deletes the todos stored in iCloud for every Mac; pass --yes to confirm",
                );
            }
            control(service::clear_cloud(), "iCloud todo data cleared", lang)
        }
        Some("repair") => repair(lang),
        Some(other) => fail(lang, &format!("unknown todo-sync subcommand: {other}")),
    }
}

fn fail(lang: Lang, message: &str) -> ! {
    eprintln!("{}{message}", i18n::err_prefix(lang));
    exit(1);
}

fn control(result: Result<(), ControlError>, done: &str, lang: Lang) -> ! {
    match result {
        Ok(()) => {
            print_line(done);
            exit(0);
        }
        Err(e) => fail(lang, &e.to_string()),
    }
}

fn status(args: &[String]) -> ! {
    let status = service::status();
    if args.iter().any(|a| a == "--json") {
        print_line(&serde_json::to_string_pretty(&status).unwrap_or_default());
        exit(0);
    }
    let minutes_ago = |ms: u64| -> String {
        if ms == 0 {
            return "never".to_string();
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let secs = now.saturating_sub(ms) / 1000;
        match secs {
            0..=59 => format!("{secs} s ago"),
            60..=3599 => format!("{} min ago", secs / 60),
            _ => format!("{} h ago", secs / 3600),
        }
    };
    print_line(&format!(
        "sync: {}{}",
        if !status.supported {
            "not available here (macOS only, not in a Dev Instance)"
        } else if status.enabled {
            "on"
        } else {
            "off"
        },
        status
            .paused
            .as_deref()
            .map(|p| format!(" (paused: {p})"))
            .unwrap_or_default()
    ));
    if status.enabled {
        print_line(&format!("this Mac: {}", status.device_name));
        print_line(&format!(
            "todos: {} pending, {} done, {} not uploaded yet",
            status.todos, status.history, status.unsynced
        ));
        print_line(&format!(
            "last pull: {}, last push: {}",
            minutes_ago(status.last_pull_ok_ms),
            minutes_ago(status.last_push_ok_ms)
        ));
        for device in &status.devices {
            print_line(&format!(
                "  mac: {}{} (synced {})",
                device.name,
                if device.this_mac { " [this Mac]" } else { "" },
                minutes_ago(device.last_sync_ms)
            ));
        }
        if let Some(last) = &status.last_tick {
            print_line(&format!(
                "last tick: {} ({}) {}",
                last.outcome,
                last.reason,
                last.message.as_deref().unwrap_or("")
            ));
        }
    }
    if status.supported {
        let helper = &status.helper;
        print_line(&format!(
            "helper: {}{}{}{}",
            helper.state,
            if helper.installed {
                ""
            } else {
                " (app not installed)"
            },
            helper
                .version
                .as_deref()
                .map(|v| format!(", v{v}"))
                .unwrap_or_default(),
            helper
                .problem
                .as_deref()
                .map(|p| format!(", problem: {p}"))
                .unwrap_or_default(),
        ));
    }
    exit(0);
}

fn summary(report: &TickReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if report.disabled {
        parts.push("disabled".into());
    }
    if report.busy {
        parts.push("busy".into());
    }
    if report.pulled {
        parts.push("pulled".into());
    }
    if let Some(p) = report.paused {
        parts.push(format!("paused={p:?}"));
    }
    if let Some(why) = &report.unavailable {
        parts.push(format!("helper-unavailable ({why})"));
    }
    if report.offline {
        parts.push("offline".into());
    }
    if report.timed_out {
        parts.push("timed-out".into());
    }
    if report.fresh {
        parts.push("fresh".into());
    }
    parts.push(format!(
        "fetched={} merged={} pushed={} unacked={}",
        report.fetched, report.merged, report.pushed, report.unacked
    ));
    parts.join(" ")
}

fn tick(args: &[String], lang: Lang) -> ! {
    let mut reason = "manual".to_string();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--reason" => match it.next() {
                Some(r) => {
                    reason = r
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                        .take(24)
                        .collect()
                }
                None => fail(lang, "--reason needs a value"),
            },
            other => fail(lang, &format!("unknown option: {other}")),
        }
    }
    match service::run_tick(&reason) {
        // Soft outcomes (offline, no helper, paused, busy) are states, not failures: the helper
        // decides about retries itself.
        Ok(report) => {
            print_line(&summary(&report));
            exit(0);
        }
        Err(e) => fail(lang, &e.to_string()),
    }
}

/// Register the helper app as a login item (what `scripts/todo-sync-install-helper.sh` does at the
/// end of an install) and start it.
fn repair(lang: Lang) -> ! {
    let Some(app) = service::helper_app_path() else {
        fail(
            lang,
            "the sync helper app is not installed (run scripts/todo-sync-install-helper.sh)",
        );
    };
    let exe = app.join("Contents/MacOS/AskHumanTodoSync");
    match std::process::Command::new(&exe).arg("--register").output() {
        Ok(out) => {
            print_line(String::from_utf8_lossy(&out.stdout).trim());
            // 3 = approval pending in System Settings > Login Items: not an error of ours.
            exit(out.status.code().unwrap_or(1));
        }
        Err(e) => fail(lang, &format!("cannot run the helper: {e}")),
    }
}
