//! Test helper: run one test body in a child process that has a throwaway `HOME` and
//! `ASKHUMAN_HOME`, so tests of the wizard's machine-reading code never touch the real home,
//! Agent configuration, login items or keychain. Same pattern as the Antigravity tests in
//! `commands.rs`: the parent test re-executes the test binary for exactly itself.

use std::os::unix::fs::PermissionsExt;

const CHILD_ENV: &str = "ASKHUMAN_TEST_ONBOARDING_CHILD";

/// True inside the re-executed child.
pub fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-run `module_path::name` in a child process with an isolated profile and assert it passes.
/// `module_path` is the caller's `module_path!()`; its leading crate segment is stripped because
/// the test harness addresses tests relative to the crate root.
pub fn run_in_child(module_path: &str, name: &str) {
    run_in_child_with(module_path, name, |_| {});
}

/// Like [`run_in_child`], with a chance to prepare the child's `HOME` first.
pub fn run_in_child_with(module_path: &str, name: &str, prepare: impl FnOnce(&std::path::Path)) {
    let root = tempfile::tempdir().unwrap();
    // Keep the child HOME free of temporary-root aliases such as macOS /var.
    let home = root.path().canonicalize().unwrap();
    let state = home.join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    prepare(&home);
    let relative = module_path
        .split_once("::")
        .map_or(module_path, |(_, rest)| rest);
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("{relative}::{name}"), "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("HOME", &home)
        .env("ASKHUMAN_HOME", &state)
        // Never let a developer's own overrides leak into the child.
        .env_remove("CODEX_HOME")
        .env_remove(crate::development::session::PROFILE_ENV)
        .env_remove(crate::development::workbench::WINDOW_ENV)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "child `{relative}::{name}` failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    // The harness prints "running 0 tests" when the name filter matched nothing.
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "child did not run exactly the requested test `{relative}::{name}`: {stdout}"
    );
}
