//! Tabby v2 core library.
//!
//! Module map (see `docs/architecture.md`):
//!
//! - [`core`]    — session/tab/pane lifecycle, event bus.
//! - [`ipc`]     — JSON-RPC protocol layer and error codes.
//! - [`ssh`]     — russh-based SSH client.
//! - [`terminal`] — PTY I/O bridge.
//! - [`serial`]  — serial port channel.
//! - [`sync`]    — config/session sync (WebDAV + Git, self-hosted only).
//! - [`plugins`] — legacy Tabby plugin RPC bridge.

pub mod commands;
pub mod core;
pub mod error;
pub mod ipc;
pub mod plugins;
pub mod profile;
pub mod profile_health;
pub mod secret;
pub mod serial;
pub mod settings;
pub mod shell_detect;
pub mod ssh;
pub mod ssh_config;
pub mod sync;
pub mod terminal;
pub mod vault;

pub use error::{CoreError, Result};

/// Library-level version, surfaced via IPC for compatibility checks.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
