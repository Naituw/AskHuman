//! Cross-platform daemon entry point and shared server core.

pub mod ask_dedup;
pub mod away;
pub mod config_watch;
pub mod handoff;
pub mod lifecycle;
pub mod popup_focus;
pub mod popup_inbox;
pub mod recovery;
pub mod request;
pub mod restart_guard;
pub mod spawn;

/// `AskHuman daemon <sub>` 入口。永不返回（自行退出进程）。
pub fn dispatch(args: &[String]) -> ! {
    std::process::exit(server_impl::dispatch(args));
}

#[path = "runtime/mod.rs"]
mod server_impl;
