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
- **Encrypted config sync** — WebDAV or Git; exports profiles, settings, shortcuts, plugins metadata, and optional Vault entries
- **Vault** — master-password secret store; unlock in Settings → Vault or **Config sync** when syncing credentials
- **Git OAuth** — GitHub / GitLab device-flow tokens in the OS keyring
- **Broadcast input** — send keystrokes to all SSH panes in a tab (`Ctrl+Shift+B`)
- **Remote desktop** — RDP / VNC profiles with optional SSH tunnel
- **X11 forwarding** — per-session (Unix), enabled in Settings → SSH
- **Host stats** — CPU/memory/disk in the status area (separate exec channel)
- **Connection import** — Settings → Profiles → **Import connections…** (or command palette). Sources: WindTerm, Termius, OpenSSH config, CSV, PuTTY `.reg`, MobaXterm, Xshell, SecureCRT, Tabby/AeroTab export. Preview step supports **search**, grouped folders (expand/collapse), select all/invert, **batch auth** (username, password, key, agent, Vault — applied on import), match auth from existing profiles, and **overwrite** confirmation for duplicates.

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
cargo check --features desktop
cargo test --lib --features desktop
cd apps/ui && npm run check
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

1. **Engine** — configured once with **Configure / re-key** (Git/WebDAV URL, sync master password, device id). Settings live in `settings.sync`; the sync master password is stored in the **OS credential store** (default account `sync.master`), not in plain settings.
2. **Data** — each **Sync now** (or auto-sync tick) exports local state → reconciles with the remote → imports back into local stores, then refreshes the UI.

| Group | What syncs |
|-------|------------|
| Connections | SSH / RDP / VNC profiles (`profiles.sled`) |
| Appearance | Theme, window, terminal, appearance, hotkeys bundle, most other `settings` keys |
| Shortcuts | `hotkeys` |
| PluginCfg | Loaded WASM plugin list |
| Credentials | Vault entries (vault must be **unlocked**; optional auto-unlock via keyring account `sync.vault`) |

**Typical setup (Settings → Config sync):**

1. Fill Git (or WebDAV) fields and choose sync groups.
2. **Save to OS credential store** for the sync master password → **Configure / re-key**.
3. If **Credentials** is enabled: use **Vault (credential sync)** to initialize/unlock, or save the vault password to the OS store.
4. **Sync now** — expect non-zero push/pull when data differs; sidebar profiles and theme update without restart.
5. Optional: **Apply auto-sync** for periodic sync.

On launch, the app runs `bootstrapSyncEngine()` so **Sync now** works without reopening settings when the engine was configured before.

Protocol details: [docs/sync-protocol.md](docs/sync-protocol.md).

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
