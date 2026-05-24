//! AeroTab host binary.
//!
//! Until the Tauri shell lands (W2), this binary speaks the v1 JSON-RPC
//! protocol over stdin/stdout — one frame per line. That makes the entire
//! method surface usable from any embedder (Tauri, websocket bridge,
//! integration tests, scripts).
//!
//! Each line on stdin is a JSON-RPC 2.0 [`Request`]; each line on stdout is
//! a [`Response`]. Other diagnostic output goes to stderr via `tracing`.

use std::sync::Arc;

use aerotab_core::commands::{register_all, AppState};
use aerotab_core::ipc::{Dispatcher, ErrorCode, Request, Response, RpcError};
use aerotab_core::CORE_VERSION;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    tracing::info!(
        version = CORE_VERSION,
        "aerotab-core starting JSON-RPC server on stdio"
    );

    aerotab_core::core::init().await?;
    aerotab_core::ipc::init().await?;
    aerotab_core::ssh::init().await?;
    aerotab_core::terminal::init().await?;
    aerotab_core::serial::init().await?;
    aerotab_core::sync::init().await?;
    aerotab_core::plugins::init().await?;

    let dispatcher = Dispatcher::new();
    let state = AppState::new();
    register_all(&dispatcher, state);

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = tokio::io::stdout();

    let dispatcher = Arc::new(dispatcher);

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatcher.dispatch(req).await,
            Err(e) => Response {
                jsonrpc: "2.0".into(),
                id: None,
                result: None,
                error: Some(RpcError::new(ErrorCode::ParseError, e.to_string())),
            },
        };
        let mut wire = serde_json::to_vec(&resp)?;
        wire.push(b'\n');
        stdout.write_all(&wire).await?;
        stdout.flush().await?;
    }

    tracing::info!("stdin closed; exiting");
    Ok(())
}
