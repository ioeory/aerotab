# AeroTab Architecture

## High-level

```
┌─────────────────────────────────────────────┐
│  Frontend Shell (apps/ui) - Tauri webview   │
│  - Terminal: xterm.js (default) or native   │
│    termgrid (WebGL/Canvas2D) via Channel    │
│  - Connection manager / settings UI         │
│  - Sync UI (WebDAV / Git, protocol 2 oplog) │
└─────────────────┬───────────────────────────┘
                  │  JSON-RPC 2.0 over Tauri IPC
                  │  (versioned, error-coded)
┌─────────────────▼───────────────────────────┐
│  Rust Core (src-tauri)                      │
│  - core::session_manager  (tab/pane lifec.) │
│  - ipc                    (protocol layer)  │
│  - ssh                    (russh client)    │
│  - terminal               (PTY + VT engine) │
│  - serial                 (serial port)     │
│  - sync                   (BlobStore oplog) │
│  - plugins::legacy_bridge (Node child proc) │
└─────────────────────────────────────────────┘
```

## MCP integration

The optional MCP server exposes an allowlisted, permission-gated adapter over the existing core. Streamable HTTP runs on the desktop async runtime; `aerotab-mcp` provides stdio integration. Listener settings and permissions are device-local, and authentication tokens remain in the OS keyring. Non-loopback listeners require explicit consent and TLS. See [mcp.md](mcp.md).

## Threading model

- **Main thread**: Tauri runtime + UI event pump.
- **Async runtime**: single `tokio` multi-thread runtime owned by the host;
  all I/O (SSH, sync, plugins) is spawned on it.
- **PTY / serial**: blocking reads in dedicated OS threads, output funneled
  to the async runtime via bounded channels (backpressure-friendly).

## Module responsibilities

### `core`
Owns session/tab/pane state, the event bus, and persistence checkpoints.
No protocol knowledge; consumes typed events from SSH/PTY/serial modules.

### `ipc`
Defines the JSON-RPC schema (method names, request/response shapes, error
codes) and version negotiation. Every other module exposes a typed handler
registered here; no module talks to the frontend directly.

### `ssh`
russh-based client. Owns connection pool, jump host chains, known_hosts,
agent forwarding. Exposes `Channel` handles that wrap streams.

### `terminal`
PTY spawning (via `portable-pty`), resize handling, and the native engine
(`terminal::engine` on `alacritty_terminal`) that pushes damage frames over a
Tauri Channel to `apps/ui` termgrid. Default UI path is native; xterm.js
(`terminal.renderer = canvas|webgl|dom`) remains as fallback.

### `serial`
serialport-rs based channel. Parameter mapping (baud, parity, stop bits,
flow control, newline conversion) preserves Tabby v1 config schema.

### `sync`
- Protocol 2: `BlobStore` + append-only encrypted oplog (`v2/devices/…/ops`),
  snapshots with watermarks, one Git commit per cycle. See
  [sync-protocol.md](sync-protocol.md).
- Legacy `SyncBackend` per-record blobs kept for purge/migration.
- Version vectors + selective groups; Argon2id KEK + ChaCha20-Poly1305.
- Credentials never sync by default; opt-in encrypted upload.

### `plugins::legacy_bridge`
Spawns a Node child process per active plugin. Communicates via JSON-RPC
over stdio. Translates v1 plugin API surface to v2 typed events.

## Error model

All cross-module errors flow through a single `CoreError` enum:

```
CoreError
├── Io(std::io::Error)
├── Protocol(IpcProtocolError)
├── Ssh(SshError)
├── Sync(SyncError)
├── Plugin(PluginError)
└── Other(anyhow::Error)
```

IPC serializes these as JSON-RPC errors with stable numeric codes (see
`src-tauri/src/ipc/mod.rs`).

## Persistence

- Config & profiles: `sled` keyed by stable IDs, with JSON export for migration.
- Secrets: OS keychain via `keyring`; only opaque IDs hit `sled`.
- Sync state: separate `sled` tree, holds version vectors and last-seen hashes.
