//! Session / tab / pane lifecycle and the in-process event bus.
//!
//! W1 stub: types only; real implementation lands W3-W4 (plan step 7).

pub mod session_manager;

pub use session_manager::SessionManager;

pub async fn init() -> crate::Result<()> {
    tracing::debug!("core::init (stub)");
    Ok(())
}
