<p align="center">
  <img src="docs/assets/logo.png" alt="AeroTab logo" width="160" />
</p>

<h1 align="center">AeroTab</h1>

<p align="center">
  Cross-platform SSH, serial, and terminal client — Rust + Tauri 2 + Svelte 5.
</p>

**AeroTab** is an independent desktop terminal. It is **not** the official [Tabby](https://github.com/Eugeny/tabby) application; it is a native rewrite focused on fast startup, low memory use, and self-hosted config sync.

## Features

- **SSH & serial** — profiles, jump chains, agent / key / password / **Vault** auth, known_hosts
- **Pane layout** — splits, drag-reorder, maximize, session workspaces
- **SFTP** — dual-pane local/remote browser, drag-and-drop, pause/resume transfers
- **Encrypted config sync** — Git, WebDAV, S3-compatible object storage, or Google Drive app data; profiles, settings, shortcuts, plugin metadata, and opt-in Notes / Vault entries
- **AI chat** — multiple OpenAI-compatible, Anthropic, and Ollama providers; explicit context selection, external MCP tools, and one bound visible terminal with Observer / Confirm / explicitly authorized Auto modes
- **Workbench** — resizable primary navigation and docked / floating Files, Notes, system, and AI panels around session panes
- **MCP automation** — permission-scoped notes / workspace context and locally approved, bounded-concurrency script batches
- **Terminal views & diagnostics** — WebView termgrid / native GPU view switching, xterm fallbacks, stable resize / scrollbar geometry, pointer-captured sidebar / panel resizing, and JSON diagnostic export
- **Vault** — master-password secret store; unlock in Settings → Vault or **Config sync** when syncing credentials
- **Git OAuth** — GitHub / GitLab device-flow tokens in the OS keyring
- **Broadcast input** — send keystrokes to all SSH panes in a tab (`Ctrl+Shift+B`)
- **Remote desktop** — RDP / VNC profiles with optional SSH tunnel
- **X11 forwarding** — per-session (Unix), enabled in Settings → SSH
- **Host stats** — CPU/memory/disk in the status area (separate exec channel)
- **Connection import** — Settings → Profiles → **Import connections…** (or command palette). Sources: WindTerm, Termius, OpenSSH config, CSV, PuTTY `.reg`, MobaXterm, Xshell, SecureCRT, Tabby/AeroTab export. Preview step supports **search**, grouped folders (expand/collapse), select all/invert, **batch auth** (username, password, key, agent, Vault — applied on import), match auth from existing profiles, and **overwrite** confirmation for duplicates.

## What's new in v0.3.3

- **Profile picker splits** — right-click an SSH profile (or use the keyboard context menu) to open it to the right or below the active pane in the current terminal tab. Connections use backend-resolved profile settings; if the target pane disappears while connecting, the unattached session is closed.
- **Focused SFTP dock** — docked file browsers show only the remote host and skip local-directory initialization; the modal browser retains its local / remote dual-pane layout.
- **Netcatty-style system details** — CPU, memory, root-disk capacity, and network metric cards with recent sampled trends, per-core usage, grouped mount capacity, interface counters, and disk I/O. Network overview excludes loopback and common virtual / duplicate interfaces; it shows sampled throughput, not bandwidth utilization. Sampling uses a separate SSH exec channel and monotonic intervals; failures, pauses, reboots, and endpoint changes break trend / counter continuity instead of inventing rates. Missing values stay unavailable, and failures retain the last good sample. Process memory rankings are not implemented.
- **Validation boundary** — automated helper / static-layout tests and successful builds do not establish real Windows WebView2 behavior; the target Windows environment still needs manual verification.

## What's new in v0.3.2

- **Profile drawer scaling isolation** — render the profile editor outside the scaled application shell so its header, scrollable form, and Cancel / Save footer stay within the viewport across UI scale settings.
- **Older WebView2 viewport fallback** — provide a `vh` fallback when dynamic viewport units are unavailable. Layout checks were performed in embedded Chromium at 800×480, device scale 1.5, and UI scales 0.6 / 1 / 1.25 / 1.4; the target Windows 10 WebView2 environment has not yet been manually verified.

## What's new in v0.3.1

- **Persistent SSH group defaults** — share username, port, authentication, proxy, and jump-chain settings across nested groups. Hosts opt in per field; explicit host settings override group defaults, and legacy profiles keep their existing settings. Group authentication uses SSH agent, Vault references, or key paths without plaintext passphrases.
- **Profile editor layout fix** — compact typography and spacing, a separately scrollable form, and fixed header / Cancel / Save actions keep lower fields accessible in smaller windows.

## Quick start

### Prerequisites

- Rust stable (see `rust-version` in [Cargo.toml](Cargo.toml))
- Node.js 20.19+ (frontend)
- Linux desktop build: `libssl-dev`, `pkg-config`, `libudev-dev`

On **Wayland** (GNOME/KDE/Sway), the main app is supported. SSH **X11 forwarding** requires [XWayland](docs/wayland.md) (`xorg-xwayland` on Arch). See [docs/wayland.md](docs/wayland.md) for limits (native embed, transparency).

### Development

```bash
cd apps/ui && npm install && npm run dev
# In another terminal:
cd src-tauri && cargo run --bin aerotab-app --features desktop
```

### Checks

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo check --features desktop
cargo clippy --workspace --all-targets --features desktop -- -D warnings
cargo test --lib --features desktop -- --test-threads=1
cd apps/ui && npm run check && npm run build
```

### Windows installer (WSL cross-compile)

```bash
./tools/build-windows-xwin.sh
# Artifact:
# target/x86_64-pc-windows-msvc/release/bundle/nsis/AeroTab_<version>_x64-setup.exe
```

Copy the installer to a Windows local path before installing (not from `\\wsl.localhost\...`). See [docs/release.md](docs/release.md) and [AGENTS.md](AGENTS.md).

## Config sync (overview)

Sync has two layers:

1. **Engine** — configure a Git, WebDAV, S3-compatible, or Google Drive target with a sync master password. New setup uses protocol 3; saved protocol-2 configurations stay on protocol 2 until explicitly migrated. Settings live in `settings.sync`; the master password is stored in the **OS credential store** (default account `sync.master`), not in plain settings.
2. **Data** — each **Sync now** (or auto-sync tick) exports local state → reconciles with the remote → imports back into local stores, then refreshes the UI.

| Group | What syncs |
|-------|------------|
| Connections | SSH / RDP / VNC profiles (`profiles.sled`) |
| Appearance | Theme, window, terminal, appearance, hotkeys bundle, most other `settings` keys |
| Shortcuts | `hotkeys` |
| PluginCfg | Loaded WASM plugin list |
| Credentials | Vault entries (opt-in; vault must be **unlocked**; optional keyring auto-unlock via `sync.vault`) |
| Notes | Ordinary notes, folders, tags, and profile associations (opt-in; not Vault secrets) |

**Typical setup (Settings → Config sync):**

1. Choose a provider and configure its target / credentials. S3 needs an existing bucket, HTTPS endpoint, region, and access credentials; Google Drive needs your OAuth client ID and browser authorization (appDataFolder only).
2. Save the sync master password to the OS credential store and configure the engine; test the connection, inspect the first merge preview, and explicitly approve it before the first sync / auto-sync in the new setup wizard.
3. Enable **Notes** and / or **Credentials** only if wanted. Credential sync requires an unlocked Vault; save unsaved note drafts before Notes sync / preview.
4. **Sync now** — exports and merges selected groups, then refreshes the UI. Target / group changes can require a fresh preview and approval.
5. Optional: enable periodic auto-sync after setup approval. Existing configurations can be restored on launch from the keyring; missing credentials or approvals still block sync.

The implemented cloud backends are **S3-compatible** storage and **Google Drive app data**, not a generic connector for Dropbox, OneDrive, Azure Blob, or arbitrary cloud drives. Provider tests and mock-backed coverage are not evidence of a successful live account round-trip.

On launch, the app runs `bootstrapSyncEngine()` so **Sync now** works without reopening settings when the engine was configured before.

Protocol details: [docs/sync-protocol.md](docs/sync-protocol.md).

## AI and terminal safety (0.3.0)

Configure providers in Settings → AI; API keys stay in the OS keyring. Presets include OpenAI, Anthropic, Ollama, DeepSeek (OpenAI-compatible), and a custom OpenAI-compatible endpoint. Remote endpoints require HTTPS; loopback HTTP is supported for local services such as Ollama.

- Bind one visible terminal and explicitly authorize output reads. **Observer** is read-only; **Confirm** approves each exact terminal input / interrupt; **Auto** requires explicit authorization for that bound session. Changing terminal, revoking access, or starting a new conversation clears authorization.
- External MCP tool calls still need their own approval; terminal Auto does not authorize arbitrary external tools. Treat terminal output, notes, resources, and prompts as untrusted context.
- Configurable execution budgets default to **12 model rounds / 30 tool calls / 3 consecutive waits** (not billing limits). Exhaustion ends tool execution and requests a summary; it does not terminate an already-running shell command. Terminal reads / waits do not establish an exit code or prove success.
- WebView shared-core rendering and native GPU surfaces are distinct views; platform availability and overlay / fallback rules apply. Wayland native embedding remains unsupported. Native-surface drag behavior and Windows Sandbox smoke results must be verified on the target machine, not inferred from unit tests or a successful build.
- Export diagnostics from Application settings or the command palette. The JSON includes sanitized failures and bounded terminal geometry events, not a terminal transcript; review it before sharing.

## Local data

| Platform | App data directory |
|----------|-------------------|
| Linux / WSL (Linux build) | `~/.local/share/com.aerotab/` |
| Windows | `%APPDATA%\com.aerotab\` |
| macOS | `~/Library/Application Support/com.aerotab/` |

Main files: `settings.sled`, `profiles.sled`, `vault.sled`, `known_hosts.json`, `plugins/`.

**OS keyring** (service `com.aerotab`): sync master password (`sync.master` by default), optional vault password for sync (`sync.vault`), Git OAuth tokens.

## Versioning & release

Bump version in this order, then rebuild installers:

1. [Cargo.toml](Cargo.toml) — `[workspace.package].version`
2. [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json) — `"version"` and window `title`
3. [apps/ui/package.json](apps/ui/package.json) — `"version"`

For **0.3.0**, keep workspace entries in `Cargo.lock`, frontend package-lock metadata, and the footer `buildId` in `App.svelte` aligned as well. Run the formatting, lint, test, frontend, and isolated Windows smoke preflight in the release playbook; report Sandbox unavailability as a blocker rather than bypassing isolation.

Tag releases `vX.Y.Z`. GitHub Actions attaches **deb** (amd64 + arm64), **Arch `.pkg.tar.zst`** (x86_64 + aarch64), **Windows NSIS**, and **macOS DMG** to each release. Full signing and feed steps: [docs/release.md](docs/release.md).

## Upgrading from Tabby v2 / older AeroTab

AeroTab **0.2.0+** uses application id **`com.aerotab`**. On first launch, if the new data directory is empty, the app copies legacy data from **`org.tabby.v2`** (see [src-tauri/src/migrate.rs](src-tauri/src/migrate.rs)).

- Windows install path: `%LOCALAPPDATA%\Programs\AeroTab\` (NSIS).
- Re-save **sync master password** and **Git OAuth** tokens in the OS keyring if prompts appear (`com.aerotab`; reads may fall back to `org.tabby.v2` once).

## Project layout

```
apps/ui/              Svelte 5 frontend (Tauri webview)
  src/lib/syncConfig.ts          Sync bootstrap & engine configure helpers
  src/lib/applyStoredSettings.ts  Re-apply theme/hotkeys after sync
src-tauri/            Rust core + Tauri host (aerotab-core)
  src/sync/bridge.rs             Export/import local stores ↔ sync engine
docs/                 Architecture, sync protocol, release
tools/                Windows build, icons, smoke install
```

## Relationship to Tabby

[Tabby](https://github.com/Eugeny/tabby) (Eugeny) is licensed under the **MIT License**. AeroTab shares some UX goals and optional v1 plugin protocol compatibility ideas but is a **separate codebase** (Rust/Tauri, not Electron). See [NOTICE](NOTICE) for third-party dependencies.

## License

[MIT](LICENSE) — Copyright (c) 2025 AeroTab Contributors.
