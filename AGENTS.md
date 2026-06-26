# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project Shape

AeroTab is a Rust + Tauri 2 desktop app with a Svelte 5 frontend.

- Frontend shell: [apps/ui](apps/ui)
- Rust/Tauri backend: [src-tauri](src-tauri)
- Architecture notes: [docs/architecture.md](docs/architecture.md)
- Wayland session notes: [docs/wayland.md](docs/wayland.md)
- Sync protocol: [docs/sync-protocol.md](docs/sync-protocol.md)
- Release playbook: [docs/release.md](docs/release.md)
- Performance benchmark notes: [docs/perf-benchmark.md](docs/perf-benchmark.md)

The README status text may lag behind active implementation work. Prefer the current package versions, command registry, and source files when they disagree with older roadmap prose.

## Commands

Run from the repository root unless noted.

**Before every commit/push to GitHub** (CI fails if skipped): `cargo fmt --all` then `cargo fmt --all -- --check`. Unformatted Rust is the most common CI failure on all three `rust-*` jobs; `cargo check` alone is not enough.

- Rust format (apply): `cargo fmt --all`
- Rust format check: `cargo fmt --all -- --check`
- Rust lint: `cargo clippy --workspace --all-targets --features desktop -- -D warnings`
- Rust tests: `cargo test --lib --features desktop -- --test-threads=1`
- Rust desktop check: `cargo check --features desktop`
- Frontend check: `cd apps/ui && npm run check`
- Frontend build: `cd apps/ui && npm run build`
- Tauri Linux deb: `cd src-tauri && cargo tauri build --bundles deb`
- **Arch Linux pkg (native)**：x86_64 用 `archlinux:base-devel` 容器；aarch64 用 `menci/archlinuxarm:base-devel`（官方 Arch 镜像仅 amd64）。执行 `./tools/build-arch-pkg.sh [version] [x86_64|aarch64]`（见 [docs/release.md](docs/release.md) §2b）
- **Windows NSIS from WSL/Linux (交叉编译)**：在仓库根目录执行 `./tools/build-windows-xwin.sh`（见下文 §WSL → Windows）

The frontend requires Node 20+. CI uses stable Rust with `rustfmt` and `clippy`; Linux builds need `libssl-dev`, `pkg-config`, and `libudev-dev`.

## Architecture Boundaries

- JSON-RPC methods are registered in [src-tauri/src/commands.rs](src-tauri/src/commands.rs). Keep frontend calls, shared TS types, and Rust params/results aligned.
- Frontend RPC access goes through [apps/ui/src/lib/rpc.ts](apps/ui/src/lib/rpc.ts). Shared UI-facing shapes live in [apps/ui/src/lib/types.ts](apps/ui/src/lib/types.ts).
- Tauri desktop shell behavior lives in [src-tauri/src/bin/aerotab-app.rs](src-tauri/src/bin/aerotab-app.rs), including updater, file pickers, transparency, and tray integration. **Windows HTML5 drag-and-drop** (pane reorder, SFTP, in-app drags) requires `"dragDropEnabled": false` on the main window in [tauri.conf.json](src-tauri/tauri.conf.json); Tauri’s default native file-drop handler blocks WebView2 DnD and shows a prohibited cursor.
- SSH, SFTP, and host stats belong under [src-tauri/src/ssh](src-tauri/src/ssh). Host stats must use a separate exec channel and must not write probe commands into terminal scrollback.
- Terminal behavior belongs in [apps/ui/src/components/TerminalPane.svelte](apps/ui/src/components/TerminalPane.svelte) and [src-tauri/src/terminal](src-tauri/src/terminal). Keep ended-session scrollback visible; do not cover the terminal with a full-screen overlay—append a small `[session ended]` marker and use a compact close/status chip only.
- Pane layout is a per-tab tree (`PaneNode` leaves and splits), not a single global split direction for the whole tab. Use [apps/ui/src/lib/tabs.svelte.ts](apps/ui/src/lib/tabs.svelte.ts) and pane-tree helpers; `tabs.movePane` reorders leaves within a tab via side targets and a small drag handle so text selection is not treated as pane drag.
- SSH UI opens profiles through `session.openSshProfile`. Keep [apps/ui/src/lib/types.ts](apps/ui/src/lib/types.ts) auth shapes aligned with Rust (`PublicKey.key_path`, etc.). `AuthMethod::Agent` uses the system ssh-agent (`SSH_AUTH_SOCK` on Unix; `\\.\pipe\openssh-ssh-agent` or `SSH_AUTH_SOCK` on Windows). PuTTY Pageant window-message protocol is not supported yet.
- SFTP lives in [apps/ui/src/components/SftpBrowser.svelte](apps/ui/src/components/SftpBrowser.svelte) (modal or per-tab right dock). Dual-pane **local + remote** browser; drag between panes or from the OS to upload/download. Text edit for remote files under 512 KiB. Transfers support pause/resume. Per-tab dock follows the tab’s **active SSH pane** (`activePaneId`); width is draggable (persisted as `sftp.dockWidthPx`). Tauri helpers: `local_list_dir`, `local_home_dir`, etc. in [aerotab-app.rs](src-tauri/src/bin/aerotab-app.rs). Shortcuts: open dock `Ctrl+Alt+F`, toggle/collapse `Ctrl+Alt+E`.
- Tab context menu: close / close others / close to right / close all / duplicate tab / open SFTP (see [TabBar.svelte](apps/ui/src/components/TabBar.svelte)).
- Jump chain: ProfileModal accepts `user@host` lines and `@profile-id` / `@Profile Name` references ([jumpProfiles.ts](apps/ui/src/lib/jumpProfiles.ts)). `~/.ssh/config` `ProxyJump` is parsed in [ssh_config.rs](src-tauri/src/ssh_config.rs) and applied when connecting from the picker ([sshConfigJump.ts](apps/ui/src/lib/sshConfigJump.ts)).
- SSH tunnels: `tunnel.open` / `tunnel.close` / `tunnel.list` in [ssh/tunnel.rs](src-tauri/src/ssh/tunnel.rs) (`-L` local, `-R` remote, `-D` SOCKS5). Manage from Settings → SSH → Port forwarding.
- **Config sync (two layers)** — (1) **Engine**: `sync.configureGit` / `sync.configureWebdav` + sync master password; persisted non-secrets in `settings.sync` (`key: sync`); master password in OS keyring via `secret.*` (default account `sync.master`). Frontend helpers: [apps/ui/src/lib/syncConfig.ts](apps/ui/src/lib/syncConfig.ts) (`bootstrapSyncEngine`, `configureSyncEngineFromSettings`, `ensureSyncEngineConfigured`). App calls `bootstrapSyncEngine` on mount ([App.svelte](apps/ui/src/App.svelte)). (2) **Data**: [src-tauri/src/sync/bridge.rs](src-tauri/src/sync/bridge.rs) `export_locals` / `import_locals` run inside `run_sync_now_cycle` before/after `sync_groups` ([commands.rs](src-tauri/src/commands.rs) `sync.now`, auto-sync). Maps: Connections → `ProfileStore`; Appearance → nearly all `settings` keys (redacts `sync`/`ai` secrets in export); Shortcuts → `hotkeys`; PluginCfg → WASM plugin list; Credentials → `VaultStore` entries when unlocked. UI: [ConfigSyncSection.svelte](apps/ui/src/components/settings/sections/ConfigSyncSection.svelte). After successful `sync.now`, call `onSyncApplied` → [applyStoredSettings.ts](apps/ui/src/lib/applyStoredSettings.ts) + sidebar refresh ([App.svelte](apps/ui/src/App.svelte) `refreshAppFromSettingsStore`). **Vault for sync**: unlock in Config sync (Credentials group); keyring account `sync.vault` (persisted `vaultKeyringAccount`); RPC `sync.ensureVaultUnlock`; auto-unlock before export when Credentials selected. Credentials group is off by default—require explicit user opt-in. Git HTTPS needs `git2` features `https` + `vendored-openssl` in root [Cargo.toml](Cargo.toml). Git OAuth: device-flow (`sync.oauth*`); tokens in keyring; `configureGit` accepts `oauth_provider` (`github` / `gitlab`).
- **SSH Vault auth** — `AuthMethod::VaultRef { entry_id, passphrase_entry_id? }` in [ssh/mod.rs](src-tauri/src/ssh/mod.rs); resolved at connect via [ssh/vault_resolve.rs](src-tauri/src/ssh/vault_resolve.rs) (`materialize_ssh_profile` in commands). ProfileModal offers “From Vault” when vault is unlocked.
- Broadcast input: per-tab mode sends keystrokes to all SSH panes via `session.writeMany` ([broadcast.ts](apps/ui/src/lib/broadcast.ts)); toggle from command palette or hotkey `toggle-broadcast` (`Ctrl+Shift+B` default).
- Remote desktop profiles (`kind: rdp | vnc`) open the system viewer via `remote.openProfile` / `remote.open` ([remote.rs](src-tauri/src/remote.rs)); optional SSH `-L` tunnel via `ssh_profile_id` on the profile spec.
- X11 forwarding: enable in Settings → SSH; `session.openSsh` reads `settings.ssh.x11Forwarding` / `x11Display` (Unix server channel only).
- Profiles carry optional `schemaVersion`, `tags`, `icon`, and `favorite` with legacy defaults. `profile.healthCheck` returns sanitized endpoint/auth/key/known_hosts checks plus optional live probe.
- Session workspaces: command palette can save/open/delete `sessionWorkspaces`; snapshots preserve pane trees and per-tab SFTP docks and replay via `tabs.addLayout` with fresh session IDs.
- Diagnostics: [apps/ui/src/lib/diagnostics.svelte.ts](apps/ui/src/lib/diagnostics.svelte.ts) records local sanitized failures only (method/source/message/timestamp—no RPC params). Application settings and the command palette can export JSON diagnostics; palette search uses hidden keywords for profile group/tag/host and includes sync actions.
- Settings UI uses section components under [apps/ui/src/components/settings](apps/ui/src/components/settings) and the coordinator in [apps/ui/src/lib/settingsStore.svelte.ts](apps/ui/src/lib/settingsStore.svelte.ts). For a11y, settings rows may use `<label class="row"><span class="row-label">…</span>…</label>` around controls without changing layout.
- i18n infrastructure is in [apps/ui/src/lib/i18n.svelte.ts](apps/ui/src/lib/i18n.svelte.ts). `application.locale` is `system`, `en`, or `zh-CN`; preview language immediately from Application settings. Add user-facing strings there instead of hardcoding new high-visibility text.
- Sidebar visibility is persisted in window settings and toggled with `Ctrl+Alt+S`.
- System tray reads `window.trayEnabled` / `trayMinimizeToTray` in [src-tauri/src/bin/aerotab-app.rs](src-tauri/src/bin/aerotab-app.rs) (`tray-icon`); close/minimize can hide to tray. `.gitignore` should keep `Cargo.lock` trackable and ignore `.tauri/dev-updater.key`.
- Terminal transfers (experimental): when enabled in terminal settings, a `trzsz` filter bridges the session with Tauri native file APIs (`pick_open_files`, `local_read_chunk`, `local_write_chunk`, etc. in [src-tauri/src/bin/aerotab-app.rs](src-tauri/src/bin/aerotab-app.rs)); drag files onto the terminal or answer a remote `trz`/`tsz` prompt to transfer. ZMODEM/lrzsz still shows an in-pane hint with an optional SFTP shortcut—native rz/sz is not implemented.

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

`target/x86_64-pc-windows-msvc/release/bundle/nsis/AeroTab_<version>_x64-setup.exe`

**手测（发版默认流程，无需再向用户确认）**：

1. 将安装包拷到 Windows 本机盘（勿从 `\\wsl.localhost\...` 直接安装）：
   `cp "target/x86_64-pc-windows-msvc/release/bundle/nsis/AeroTab_<ver>_x64-setup.exe" /mnt/c/Users/<user>/Downloads/`
2. 静默安装并重装覆盖：`tools/install-windows-smoke.sh`（或手动 `setup.exe /S`）
3. 冒烟：启动已安装应用，确认进程存在、窗口标题含版本号；必要时检查 footer buildId。

详见 [docs/release.md](docs/release.md) §3b（Authenticode 需在 Windows 上单独 `signtool`；updater 用同目录 `.exe.sig`）。

## Repo Conventions And Pitfalls

- Preserve user changes. Check `git status --short` before edits and do not revert unrelated work.
- Prefer focused changes that match nearby patterns over broad refactors.
- For Windows-facing fixes, bump the release version in **three places** (see [docs/release.md](docs/release.md)): root [Cargo.toml](Cargo.toml) `[workspace.package].version`, [src-tauri/tauri.conf.json](src-tauri/tauri.conf.json) `version` + window `title`, [apps/ui/package.json](apps/ui/package.json). Optionally bump `buildId` in [App.svelte](apps/ui/src/App.svelte). CI release job artifact paths in [.github/workflows/ci.yml](.github/workflows/ci.yml) must match the version string.
- When editing [Cargo.lock](Cargo.lock), change only this workspace package entry for release bumps. Do not blanket-replace dependency versions that happen to match.
- Windows 包在 WSL 上一律走 `./tools/build-windows-xwin.sh`（`cargo-xwin` + `makensis`），不要改用裸 MSVC 链路。
- When invoking PowerShell from bash, escape `$` variables so bash does not expand them first.
- Copy NSIS installers to a Windows local path before silent install testing; installing directly from `\\wsl.localhost` can hang.
- Windows transparency: set WebView2 background in [src-tauri/src/bin/aerotab-app.rs](src-tauri/src/bin/aerotab-app.rs), use one painted CSS alpha layer only, and xterm `allowTransparency` with DOM renderer (avoid canvas-webgl when opacity is below 100%).
- `ssh.hostStats` uses a separate exec channel (Linux `/proc`/`df` first; macOS/BSD `sysctl`/`vm_stat`/FreeBSD fallbacks). Never write probe commands into terminal scrollback; polling follows SSH settings.
- SFTP sudo mode is intentionally passwordless only: it uses `sudo -n` and should fail fast when a password prompt would be required.
- Color scheme swatches should key by scheme plus index, not by color value; palettes can contain duplicate colors.
- App identifier is `com.aerotab`. Data dir: `~/.local/share/com.aerotab` (Linux), `%APPDATA%\com.aerotab` (Windows). Legacy Tabby v2 data under `org.tabby.v2` is copied on first launch via [`migrate.rs`](src-tauri/src/migrate.rs); keyring reads fall back to service `org.tabby.v2` when `com.aerotab` has no entry ([`secret.rs`](src-tauri/src/secret.rs)). Do not hand-edit `.sled` databases.
- Config sync UI flow: fill backend → save sync master to keyring → **Configure / re-key** → (if Credentials) vault unlock in Config sync → **Sync now**. `Configure` calls `persist()` so `settings.sync` is saved. Master password is never stored in `settings.sync`.
- **Import wizard / RPC payloads**: Do not `structuredClone()` Svelte `$state` or RPC-hydrated objects (e.g. `ImportCandidate.profile`) — WebView2 throws *could not be cloned*. Deep-copy with `JSON.parse(JSON.stringify(x))` (or field-wise clones like `cloneAuth`) before `profile.importApply`. Overwrite apply must send the frontend profile snapshot; do not rely on re-reading the WindTerm file + `source_id` lookup alone. Use inline confirm in high-z modals (`ImportConnectionsWizard` z-71), not `appConfirm`, which can sit behind the wizard.
- **Wayland (Linux)**: Main SSH/SFTP/sync flows run on Wayland via WebKitGTK. SSH X11 forwarding needs XWayland + reachable `DISPLAY` (settings `ssh.x11Display` override); connect is rejected early when enabled but unavailable. Native terminal **embed** is blocked on Wayland—use detached external terminal. Window opacity &lt;100% may be inconsistent; see [docs/wayland.md](docs/wayland.md). Session probe: Tauri `session_info` / RPC `desktop.sessionInfo`.

## Validation Expectations

Scale validation to the risk of the change.

- **Any Rust edit**: `cargo fmt --all` + `cargo fmt --all -- --check` before commit/push (required — matches CI).
- UI changes: at least `cd apps/ui && npm run check` or `npm run build`.
- Backend/RPC changes: `cargo check --features desktop`, `cargo clippy`, and relevant tests.
- Release or Windows-facing changes: `./tools/build-windows-xwin.sh` then `./tools/install-windows-smoke.sh` (无需再向用户确认是否安装冒烟).

Do not commit or create branches unless the user explicitly asks.
