# Tabby v2 Architecture

## High-level

```
┌─────────────────────────────────────────────┐
│  Frontend Shell (apps/ui) - Tauri webview   │
│  - Terminal renderer (xterm.js compatible)  │
│  - Connection manager / settings UI         │
│  - Sync UI (WebDAV / Git config + state)    │
└─────────────────┬───────────────────────────┘
                  │  JSON-RPC 2.0 over Tauri IPC
                  │  (versioned, error-coded)
┌─────────────────▼───────────────────────────┐
│  Rust Core (src-tauri)                      │
│  - core::session_manager  (tab/pane lifec.) │
│  - ipc                    (protocol layer)  │
│  - ssh                    (russh client)    │
│  - terminal               (PTY bridge)      │
│  - serial                 (serial port)     │
│  - sync                   (WebDAV + Git)    │
│  - plugins::legacy_bridge (Node child proc) │
└─────────────────────────────────────────────┘
```

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
PTY spawning (via `portable-pty`), resize handling, output batching for
high-throughput streams. Pluggable: backs both local shells and SSH channels.

### `serial`
serialport-rs based channel. Parameter mapping (baud, parity, stop bits,
flow control, newline conversion) preserves Tabby v1 config schema.

### `sync`
- Backend trait + first-party impls: `webdav`, `git`.
- Snapshot model with version vectors; selective sync groups
  (connections / appearance / shortcuts / plugin-config).
- Crypto envelope: Argon2id-derived KEK + ChaCha20-Poly1305 per-record.
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
