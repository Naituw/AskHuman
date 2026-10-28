//! `run_listing`: the way model catalogs read what an Agent CLI prints.

use super::*;

/// A stand-in for an Agent CLI that runs `body`. It is run once beforehand (with an argument that
/// makes it exit at once) so a fork in another test cannot make the first real run fail with
/// ETXTBSY.
fn listing_tool(dir: &Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\n[ \"$1\" = \"--prime\" ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..50 {
        let primed = Command::new(&path)
            .arg("--prime")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if primed.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    path.to_string_lossy().to_string()
}

#[test]
fn a_listing_gives_both_streams_whatever_the_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let tool = listing_tool(
        dir.path(),
        "tool",
        "echo 'out line'\necho 'err line' 1>&2\nexit 3",
    );
    let text = run_listing(&tool, &["models"], Duration::from_secs(20)).unwrap();
    assert!(text.contains("out line"), "{text}");
    assert!(text.contains("err line"), "{text}");
}

#[test]
fn a_listing_that_prints_nothing_is_no_answer() {
    let dir = tempfile::tempdir().unwrap();
    let quiet = listing_tool(dir.path(), "quiet", "exit 0");
    assert_eq!(run_listing(&quiet, &[], Duration::from_secs(20)), None);
    let blank = listing_tool(dir.path(), "blank", "echo '   '\nexit 0");
    assert_eq!(run_listing(&blank, &[], Duration::from_secs(20)), None);
}

#[test]
fn a_listing_longer_than_a_pipe_buffer_is_read_through_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    // About 3 MB: far more than a pipe holds, and more than the cap.
    let tool = listing_tool(
        dir.path(),
        "long",
        "yes 'model-x - A model with a name that makes every line long enough to matter ......' | head -n 40000",
    );
    let text = run_listing(&tool, &[], Duration::from_secs(60)).unwrap();
    assert!(text.len() <= LISTING_MAX_BYTES, "{}", text.len());
    assert!(text.len() > LISTING_MAX_BYTES / 2, "{}", text.len());
}

#[test]
fn a_listing_still_running_at_the_deadline_is_killed_and_gives_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // `exec` so the sleeper itself is what gets killed.
    let tool = listing_tool(dir.path(), "slow", "echo started\nexec sleep 30");
    let started = std::time::Instant::now();
    assert_eq!(run_listing(&tool, &[], Duration::from_millis(600)), None);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_process_the_command_left_behind_does_not_hold_the_answer_back() {
    let dir = tempfile::tempdir().unwrap();
    let tool = listing_tool(dir.path(), "lingering", "echo listed\nsleep 4 &\nexit 0");
    let started = std::time::Instant::now();
    let text = run_listing(&tool, &[], Duration::from_secs(20)).unwrap();
    assert!(text.contains("listed"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn listing_arguments_and_the_executable_are_words_and_never_shell_syntax() {
    let dir = tempfile::tempdir().unwrap();
    let tool = listing_tool(
        dir.path(),
        "it's a tool",
        "for arg in \"$@\"; do echo \"arg=$arg\"; done",
    );
    let text = run_listing(
        &tool,
        &["--list-models", "a b", "x;echo pwned", "$HOME"],
        Duration::from_secs(20),
    )
    .unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.contains(&"arg=--list-models"), "{text}");
    assert!(lines.contains(&"arg=a b"), "{text}");
    assert!(lines.contains(&"arg=x;echo pwned"), "{text}");
    assert!(lines.contains(&"arg=$HOME"), "{text}");
    assert!(!lines.contains(&"pwned"), "{text}");
}

#[test]
fn a_listing_command_that_cannot_start_is_no_usable_answer() {
    let text = run_listing(
        "/nonexistent/askhuman-no-such-agent",
        &["models"],
        Duration::from_secs(20),
    );
    // The shell may say why on stderr; no model parser accepts that.
    if let Some(text) = text {
        assert!(!text.contains("Available models"), "{text}");
    }
}
