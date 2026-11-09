# AeroTab Architecture

## High-level

```
┌─────────────────────────────────────────────┐
│  Frontend Shell (apps/ui) - Tauri webview   │
│  - Terminal: termgrid / native GPU views    │
│    with xterm.js renderer fallbacks         │
│  - PrimaryNav / Workbench / Notes / AI      │
│  - Sync UI (Git/WebDAV/S3/Drive, v2/v3)      │
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

The legacy renderer value `native` means the **WebView shared-core termgrid**, not an OS GPU surface. A per-pane `native-surface` choice uses `NativeTerminalView` / `nativeViewHost` to attach a native GPU view to the shared engine. View handoffs retain the source until the destination is ready; generation / revision checks guard stale work and preserve the source on failure. Overlay occlusion and floating Workbench panels can require a WebView fallback. Wayland native embedding remains unsupported.

Geometry uses measured host sizes and bounded fit / draw diagnostics to avoid scrollbar-induced resize loops, including fractional shell zoom dimensions. Terminal selection uses pointer capture; splitter gestures handle cancellation / capture loss. These implementation paths and automated regressions do not establish that native-surface dragging has been manually validated on every platform.

### Frontend shell and context

`PrimaryNav.svelte` provides collapsible, resizable navigation for Hosts, Sessions, Files, monitoring, keychain, proxies, forwarding, scripts, notes, known hosts, and logs. `Workbench.svelte` lays out terminal content with docked or floating Files, Notes, system, and additional AI panels, with persistent local layout, side ordering, span resizing, and cancellable drag gestures. Session pane trees remain separate from Workbench panel placement.

Ordinary Notes have folders, tags, profile associations, and revision / version checks. They are not a secret store. The diagnostics exporter includes sanitized local failures and a bounded geometry-only event ring; it does not capture terminal transcripts.

### `ai`, `ai_providers`, `ai_terminal`, `mcp_client`

AI is disabled by default. Provider configurations support OpenAI-compatible endpoints (including DeepSeek / custom), Anthropic, and Ollama, with multiple saved providers and OS-keyring API keys. The frontend selects provider / model, streams chat, chooses context, and displays exact-call approvals. Outbound MCP clients expose tools, resources, resource templates, and prompts without broadening the inbound server's permissions.

`ai_terminal` binds each generation to one explicitly selected, already-visible live terminal, never a separate SSH exec channel. Output reads require authorization. Observer exposes only reads / waits; Confirm obtains exact-call approval before input or Ctrl+C; Auto requires explicit authorization tied to the bound session and is carried on each generation request. External MCP calls still require approval. Changing the bound terminal, revoking access, or creating a new conversation clears the frontend grant.

Tool-loop budgets default to 12 rounds, 30 calls, and 3 consecutive waits and are configurable (ranges 2–100, 1–200, and 1–30 respectively). Budget exhaustion requests a final summary, not shell termination. Terminal tools return unknown command status / no exit code; output or elapsed waiting must never be presented as proof of command completion.

### `serial`
serialport-rs based channel. Parameter mapping (baud, parity, stop bits,
flow control, newline conversion) preserves Tabby v1 config schema.

### `sync`
- `BlobStore` backends: Git, WebDAV, S3-compatible storage, and Google Drive appDataFolder. Protocol 3 is the new-configuration default; protocol 2 stays available for saved configurations and explicit migration. See [sync-protocol.md](sync-protocol.md).
- Protocol 3 uses isolated `v3/` append-only operations, immutable per-writer snapshot publications, group-scoped coverage, and peer acknowledgements / retirement; no shared latest-snapshot pointer.
- `bridge` exports / imports profiles, settings, plugins, optional Vault entries, and per-entity Notes. Notes sync pauses around dirty drafts and preserves conflict copies rather than silently replacing edits.
- First-merge previews and approval are bound to the configured target; maintenance includes explicit migration and encrypted local recovery. Device identity and projection / maintenance journals remain local.
- Version vectors + selective groups; Argon2id KEK + ChaCha20-Poly1305. Credentials and Notes are both opt-in, off by default.

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
