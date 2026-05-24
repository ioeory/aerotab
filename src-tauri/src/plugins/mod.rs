//! Legacy plugin compatibility bridge.
//!
//! v1 legacy plugins run as Node child processes; we communicate over stdio
//! using the same JSON-RPC envelope as the frontend IPC, with a translation
//! layer that maps v1 lifecycle hooks to v2 typed events.
//!
//! Priority plugin list locked in for v2 GA (see session plan Decisions):
//!   docker, sync-config, quick-cmds, save-output, workspace-manager,
//!   background, highlight.
//!
//! Real implementation lands W13-W14 (plan step 12).

pub mod legacy_bridge;
pub mod wasm_host;

pub async fn init() -> crate::Result<()> {
    tracing::debug!("plugins::init (stub)");
    Ok(())
}
