//! JSON-RPC 2.0 protocol layer between the Tauri shell and the Rust core.
//!
//! W1 ships only the envelope types and the error-code table. The full
//! method registry is frozen in W3 (plan step 6).

use serde::{Deserialize, Serialize};

pub mod dispatcher;
pub use dispatcher::{Dispatcher, Handler, HandlerFuture};

/// Protocol version negotiated at handshake. Bump on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Stable numeric error codes. Keep in sync with `docs/architecture.md`.
#[repr(i32)]
#[derive(Debug, Clone, Copy)]
pub enum ErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,

    // Domain-specific (Tabby reserves -32000 .. -32099)
    SessionNotFound = -32000,
    SshAuthFailed = -32010,
    SyncConflict = -32020,
    PluginCrashed = -32030,
    NotImplemented = -32099,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code as i32,
            message: message.into(),
            data: None,
        }
    }
}

pub async fn init() -> crate::Result<()> {
    tracing::debug!(protocol_version = PROTOCOL_VERSION, "ipc::init (stub)");
    Ok(())
}
