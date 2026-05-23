# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Shape

Tabby v2 is a Rust + Tauri 2 desktop app with a Svelte 5 frontend.

- Frontend shell: [apps/ui](apps/ui)
- Rust/Tauri backend: [src-tauri](src-tauri)
- Architecture notes: [docs/architecture.md](docs/architecture.md)
- Sync protocol: [docs/sync-protocol.md](docs/sync-protocol.md)
- Release playbook: [docs/release.md](docs/release.md)
- Performance benchmark notes: [docs/perf-benchmark.md](docs/perf-benchmark.md)

The README status text may lag behind active implementation work. Prefer the current package versions, command registry, and source files when they disagree with older roadmap prose.

## Commands

Run from the repository root unless noted.

- Rust format check: `cargo fmt --all -- --check`
- Rust lint: `cargo clippy --workspace --all-targets --features desktop -- -D warnings`
- Rust tests: `cargo test --lib --features desktop -- --test-threads=1`
- Rust desktop check: `cargo check --features desktop`
- Frontend check: `cd apps/ui && npm run check`
- Frontend build: `cd apps/ui && npm run build`
- Tauri Linux deb: `cd src-tauri && cargo tauri build --bundles deb`
- **Windows NSIS from WSL/Linux (交叉编译)**：在仓库根目录执行 `./tools/build-windows-xwin.sh`（见下文 §WSL → Windows）

The frontend requires Node 20+. CI uses stable Rust with `rustfmt` and `clippy`; Linux builds need `libssl-dev`, `pkg-config`, and `libudev-dev`.

## Architecture Boundaries

- JSON-RPC methods are registered in [src-tauri/src/commands.rs](src-tauri/src/commands.rs). Keep frontend calls, shared TS types, and Rust params/results aligned.
- Frontend RPC access goes through [apps/ui/src/lib/rpc.ts](apps/ui/src/lib/rpc.ts). Shared UI-facing shapes live in [apps/ui/src/lib/types.ts](apps/ui/src/lib/types.ts).
- Tauri desktop shell behavior lives in [src-tauri/src/bin/tabby-app.rs](src-tauri/src/bin/tabby-app.rs), including updater, file pickers, transparency, and tray integration.
- SSH, SFTP, and host stats belong under [src-tauri/src/ssh](src-tauri/src/ssh). Host stats must use a separate exec channel and must not write probe commands into terminal scrollback.
- Terminal behavior belongs in [apps/ui/src/components/TerminalPane.svelte](apps/ui/src/components/TerminalPane.svelte) and [src-tauri/src/terminal](src-tauri/src/terminal). Keep ended-session scrollback visible; do not cover the terminal with a full-screen overlay—append a small `[session ended]` marker and use a compact close/status chip only.
- Pane layout is a per-tab tree (`PaneNode` leaves and splits), not a single global split direction for the whole tab. Use [apps/ui/src/lib/tabs.svelte.ts](apps/ui/src/lib/tabs.svelte.ts) and pane-tree helpers; `tabs.movePane` reorders leaves within a tab via side targets and a small drag handle so text selection is not treated as pane drag.
- SSH UI opens profiles through `session.openSshProfile`. Keep [apps/ui/src/lib/types.ts](apps/ui/src/lib/types.ts) auth shapes aligned with Rust (`PublicKey.key_path`, etc.). `AuthMethod::Agent` uses the system ssh-agent (`SSH_AUTH_SOCK` on Unix; `\\.\pipe\openssh-ssh-agent` or `SSH_AUTH_SOCK` on Windows). PuTTY Pageant window-message protocol is not supported yet.
- SFTP lives in [apps/ui/src/components/SftpBrowser.svelte](apps/ui/src/components/SftpBrowser.svelte) (modal or per-tab right dock). Per-tab dock follows the tab’s **active SSH pane** (`activePaneId`); width is draggable (persisted as `sftp.dockWidthPx`). Sidebar/global opens use a pinned target when the pane is not SSH. Shortcuts: open dock `Ctrl+Alt+F`, toggle/collapse `Ctrl+Alt+E`.
- Tab context menu: close / close others / close to right / close all / duplicate tab / open SFTP (see [TabBar.svelte](apps/ui/src/components/TabBar.svelte)).
- Jump chain: ProfileModal accepts `user@host` lines and `@profile-id` / `@Profile Name` references ([jumpProfiles.ts](apps/ui/src/lib/jumpProfiles.ts)). `~/.ssh/config` `ProxyJump` is parsed in [ssh_config.rs](src-tauri/src/ssh_config.rs) and applied when connecting from the picker ([sshConfigJump.ts](apps/ui/src/lib/sshConfigJump.ts)).
- Sync supports selective groups on `sync.now` / `sync.startAutoSync` via `{ groups }`. Credentials sync is off by default in the UI—do not enable credential upload without explicit user intent.
- Profiles carry optional `schemaVersion`, `tags`, `icon`, and `favorite` with legacy defaults. `profile.healthCheck` returns sanitized endpoint/auth/key/known_hosts checks plus optional live probe.
- Session workspaces: command palette can save/open/delete `sessionWorkspaces`; snapshots preserve pane trees and per-tab SFTP docks and replay via `tabs.addLayout` with fresh session IDs.
- Diagnostics: [apps/ui/src/lib/diagnostics.svelte.ts](apps/ui/src/lib/diagnostics.svelte.ts) records local sanitized failures only (method/source/message/timestamp—no RPC params). Application settings and the command palette can export JSON diagnostics; palette search uses hidden keywords for profile group/tag/host and includes sync actions.
- Settings UI uses section components under [apps/ui/src/components/settings](apps/ui/src/components/settings) and the coordinator in [apps/ui/src/lib/settingsStore.svelte.ts](apps/ui/src/lib/settingsStore.svelte.ts). For a11y, settings rows may use `<label class="row"><span class="row-label">…</span>…</label>` around controls without changing layout.
- i18n infrastructure is in [apps/ui/src/lib/i18n.svelte.ts](apps/ui/src/lib/i18n.svelte.ts). `application.locale` is `system`, `en`, or `zh-CN`; preview language immediately from Application settings. Add user-facing strings there instead of hardcoding new high-visibility text.
- Sidebar visibility is persisted in window settings and toggled with `Ctrl+Alt+S`.
- System tray reads `window.trayEnabled` / `trayMinimizeToTray` in [src-tauri/src/bin/tabby-app.rs](src-tauri/src/bin/tabby-app.rs) (`tray-icon`); close/minimize can hide to tray. `.gitignore` should keep `Cargo.lock` trackable and ignore `.tauri/dev-updater.key`.
- Terminal transfers (experimental): when enabled in terminal settings, a `trzsz` filter bridges the session with Tauri native file APIs (`pick_open_files`, `local_read_chunk`, `local_write_chunk`, etc. in [src-tauri/src/bin/tabby-app.rs](src-tauri/src/bin/tabby-app.rs)); drag files onto the terminal or answer a remote `trz`/`tsz` prompt to transfer. ZMODEM/lrzsz still shows an in-pane hint with an optional SFTP shortcut—native rz/sz is not implemented.

## WSL → Windows NSIS（交叉编译，默认发布路径）

在 **WSL2** 下打 Windows 安装包时，用 [tools/build-windows-xwin.sh](tools/build-windows-xwin.sh)（内部 `cd src-tauri` 再 `cargo tauri build --runner cargo-xwin`），**不要**：

- 在仓库根目录裸跑 `cargo tauri build`（`beforeBuildCommand` 会因 cwd 错误失败）；
- 在 WSL 里依赖本机完整 MSVC / `lib.exe`（常会缺 CRT 头文件）。

**一次性依赖（WSL Debian/Ubuntu）**：

```bash
cargo install cargo-xwin
sudo apt install -y nsis    # 提供 makensis
rustup target add x86_64-pc-windows-msvc
```

**每次发版（仓库根目录）**：

```bash
./tools/build-windows-xwin.sh
```

可选：`TARGET=x86_64-pc-windows-msvc` `BUNDLES=nsis`；额外参数会传给 `cargo tauri build`（例如 `-- --features desktop`）。

**产物路径**：

`target/x86_64-pc-windows-msvc/release/bundle/nsis/Tabby v2_<version>_x64-setup.exe`

**手测（发版默认流程，无需再向用户确认）**：

1. 将安装包拷到 Windows 本机盘（勿从 `\\wsl.localhost\...` 直接安装）：
   `cp "target/x86_64-pc-windows-msvc/release/bundle/nsis/Tabby v2_<ver>_x64-setup.exe" /mnt/c/Users/<user>/Downloads/`
2. 静默安装并重装覆盖：`tools/install-windows-smoke.sh`（或手动 `setup.exe /S`）
3. 冒烟：启动已安装应用，确认进程存在、窗口标题含版本号；必要时检查 footer buildId。

详见 [docs/release.md](docs/release.md) §3b（Authenticode 需在 Windows 上单独 `signtool`；updater 用同目录 `.exe.sig`）。

## Repo Conventions And Pitfalls

- Preserve user changes. Check `git status --short` before edits and do not revert unrelated work.
- Prefer focused changes that match nearby patterns over broad refactors.
- For Windows-facing fixes, bump package/app versions so testers do not reinstall a stale same-version bundle. Update the workspace version, Tauri version/title, UI package versions, and UI build markers together.
- When editing [Cargo.lock](Cargo.lock), change only this workspace package entry for release bumps. Do not blanket-replace dependency versions that happen to match.
- Windows 包在 WSL 上一律走 `./tools/build-windows-xwin.sh`（`cargo-xwin` + `makensis`），不要改用裸 MSVC 链路。
- When invoking PowerShell from bash, escape `$` variables so bash does not expand them first.
- Copy NSIS installers to a Windows local path before silent install testing; installing directly from `\\wsl.localhost` can hang.
- Windows transparency: set WebView2 background in [src-tauri/src/bin/tabby-app.rs](src-tauri/src/bin/tabby-app.rs), use one painted CSS alpha layer only, and xterm `allowTransparency` with DOM renderer (avoid canvas-webgl when opacity is below 100%).
- `ssh.hostStats` uses a separate exec channel (Linux `/proc`/`df` first; macOS/BSD `sysctl`/`vm_stat`/FreeBSD fallbacks). Never write probe commands into terminal scrollback; polling follows SSH settings.
- SFTP sudo mode is intentionally passwordless only: it uses `sudo -n` and should fail fast when a password prompt would be required.
- Color scheme swatches should key by scheme plus index, not by color value; palettes can contain duplicate colors.

## Validation Expectations

Scale validation to the risk of the change. For UI changes, at least run `cd apps/ui && npm run check` or `npm run build`. For backend/RPC changes, run `cargo check --features desktop` and relevant tests. For release or Windows-facing changes: `./tools/build-windows-xwin.sh` then `./tools/install-windows-smoke.sh` (无需再向用户确认是否安装冒烟).

Do not commit or create branches unless the user explicitly asks.
