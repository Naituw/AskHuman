//! Explicit development verification, independent from the everyday Agent session.

pub mod build;
pub mod channel_safety;
pub mod project;
pub mod relay;
pub mod routing;
pub mod session;
pub mod state;
pub mod verification;
pub mod workbench;
pub mod workspace;

pub const CAPABILITY_VERSION: u32 = 2;

pub fn client_environment_key(key: &str) -> bool {
    key.eq_ignore_ascii_case(session::PROFILE_ENV)
        || key.eq_ignore_ascii_case(verification::VERIFICATION_ENV)
        || key.eq_ignore_ascii_case(verification::DRIVER_ENV)
        || key.eq_ignore_ascii_case(workbench::WINDOW_ENV)
}

pub fn clear_client_context(command: &mut std::process::Command) {
    command
        .env_remove(session::PROFILE_ENV)
        .env_remove(verification::VERIFICATION_ENV)
        .env_remove(verification::DRIVER_ENV);
    command.env_remove(workbench::WINDOW_ENV);
}

pub fn capabilities() -> serde_json::Value {
    serde_json::json!({
        "developmentVerification": CAPABILITY_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
    })
}

pub fn window_title(title: &str) -> String {
    match routing::inherited()
        .ok()
        .flatten()
        .filter(|t| t.kind == routing::TargetKind::Dev)
    {
        Some(target) => format!(
            "Dev · {} · {title}",
            target
                .worktree
                .as_ref()
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy()
        ),
        None => title.to_string(),
    }
}
