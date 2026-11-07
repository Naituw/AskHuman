//! Post-install setup wizard backend (`docs/specs/onboarding.md`, `docs/plans/onboarding.md`).
//!
//! The GUI window, the terminal text flow and every entry point (Settings banner, tray item,
//! popup strip, terminal welcome line) share this module: which of the wizard's domains the user
//! has already decided (`state`), which Agents exist and what applying an integration changes
//! (`agents`, `detect`, `plan`, `apply`), the remote-answering channels (`channels`), the silent
//! environment check (`env`) and the terminal version of the whole flow (`text`).

pub mod agents;
pub mod apply;
pub mod channels;
pub mod commands;
pub mod detect;
pub mod env;
pub mod hint;
pub mod plan;
pub mod probe;
pub mod probe_run;
pub mod state;
#[cfg(all(test, unix))]
pub(crate) mod testutil;
pub mod text;
