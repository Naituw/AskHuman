//! Cross-device todo sync core (spec `docs/specs/todo-sync.md`, plan `docs/plans/todo-sync.md`).
//!
//! A transport-agnostic engine: it keeps `todos.json` as the single local source of truth, mirrors it
//! into field-level-mergeable entities kept in a sidecar `state.json`, and exchanges those entities
//! with "the cloud" through a [`transport::Transport`]. Nothing in here knows about CloudKit.
//!
//! Module map: `hlc` / `order_key` (clock and order keys), `identity` (project identity from the git
//! remote), `model` (entity and state), `merge` (the lattice join), `links` (directory → identity),
//! `diff` (local edits → entities), `apply` (entities → local files), `store` (sidecar persistence),
//! `transport` (what the core needs from the cloud, plus an in-memory test double) and `tick` (the
//! orchestration, with crash-injection points). On top of the engine: `helper_client` (the production
//! transport, a client of the CloudKit helper app's RPC) and `service` (file locations, the read
//! hooks, enabling and disabling, status).
//!
//! Sync is opt-in per Mac; until it is enabled every entry point returns after one `stat`.
pub mod apply;
pub mod cloudkeys;
pub mod commands;
pub mod diff;
pub mod helper_client;
pub mod hlc;
pub mod identity;
pub mod links;
pub mod merge;
pub mod model;
pub mod order_key;
pub mod service;
pub mod store;
pub mod tick;
pub mod transport;

#[cfg(test)]
mod sim;
