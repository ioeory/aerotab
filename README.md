<p align="center">
  <img src="docs/assets/logo.png" alt="AeroTab logo" width="160" />
</p>

<h1 align="center">AeroTab</h1>

<p align="center">
  Cross-platform SSH, serial, and terminal client — Rust + Tauri 2 + Svelte 5.
</p>

**AeroTab** is an independent desktop terminal. It is **not** the official [Tabby](https://github.com/Eugeny/tabby) application; it is a native rewrite focused on fast startup, low memory use, and self-hosted config sync.

## Features

- **SSH & serial** — profiles, jump chains, agent/key/password auth, known_hosts
- **Pane layout** — splits, drag-reorder, maximize, session workspaces
- **SFTP** — dual-pane local/remote browser, drag-and-drop, pause/resume transfers
- **Config sync** — encrypted WebDAV or Git backends; optional credential groups
- **Git OAuth** — GitHub / GitLab device-flow tokens in the OS keyring
- **Broadcast input** — send keystrokes to all SSH panes in a tab (`Ctrl+Shift+B`)
- **Remote desktop** — RDP / VNC profiles with optional SSH tunnel
- **X11 forwarding** — per-session (Unix), enabled in Settings → SSH
- **Host stats** — CPU/memory/disk in the status area (separate exec channel)

## Quick start

### Prerequisites

- Rust stable (see `rust-version` in [Cargo.toml](Cargo.toml))
- Node.js 20.19+ (frontend)
- Linux desktop build: `libssl-dev`, `pkg-config`, `libudev-dev`

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

See [docs/release.md](docs/release.md) and [AGENTS.md](AGENTS.md).

## Upgrading from AeroTab

AeroTab **0.2.0+** uses a new application ID (`com.aerotab`). On first launch, the app attempts to **copy** profile and settings data from the previous `com.aerotab` data directory when the new directory is empty.

- **Install path** changes from `AeroTab` to `AeroTab` under `%LOCALAPPDATA%` (Windows).
- **OS keyring** entries use service `com.aerotab`; you may need to re-save sync master passwords and OAuth tokens once.

## Project layout

```
apps/ui/          Svelte 5 frontend (Tauri webview)
src-tauri/        Rust core + Tauri host (aerotab-core)
docs/             Architecture, sync protocol, release notes
tools/            Windows build, icons, benchmarks
```

## Relationship to Tabby

[Tabby](https://github.com/Eugeny/tabby) (Eugeny) is licensed under the **MIT License**. AeroTab shares some UX goals and optional v1 plugin protocol compatibility ideas but is a **separate codebase** (Rust/Tauri, not Electron). See [NOTICE](NOTICE) for third-party dependencies.

## License

[MIT](LICENSE) — Copyright (c) 2025 AeroTab Contributors.
