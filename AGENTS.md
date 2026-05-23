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
- Tauri Linux deb: `cargo tauri build --bundles deb`
- Windows NSIS from Linux/WSL: `tools/build-windows-xwin.sh --ci`

The frontend requires Node 20+. CI uses stable Rust with `rustfmt` and `clippy`; Linux builds need `libssl-dev`, `pkg-config`, and `libudev-dev`.

## Architecture Boundaries

- JSON-RPC methods are registered in [src-tauri/src/commands.rs](src-tauri/src/commands.rs). Keep frontend calls, shared TS types, and Rust params/results aligned.
- Frontend RPC access goes through [apps/ui/src/lib/rpc.ts](apps/ui/src/lib/rpc.ts). Shared UI-facing shapes live in [apps/ui/src/lib/types.ts](apps/ui/src/lib/types.ts).
- Tauri desktop shell behavior lives in [src-tauri/src/bin/tabby-app.rs](src-tauri/src/bin/tabby-app.rs), including updater, file pickers, transparency, and tray integration.
- SSH, SFTP, and host stats belong under [src-tauri/src/ssh](src-tauri/src/ssh). Host stats must use a separate exec channel and must not write probe commands into terminal scrollback.
- Terminal behavior belongs in [apps/ui/src/components/TerminalPane.svelte](apps/ui/src/components/TerminalPane.svelte) and [src-tauri/src/terminal](src-tauri/src/terminal). Keep ended-session scrollback visible.
- Settings UI uses section components under [apps/ui/src/components/settings](apps/ui/src/components/settings) and the coordinator in [apps/ui/src/lib/settingsStore.svelte.ts](apps/ui/src/lib/settingsStore.svelte.ts).
- i18n infrastructure is in [apps/ui/src/lib/i18n.svelte.ts](apps/ui/src/lib/i18n.svelte.ts). Add user-facing strings there instead of hardcoding new high-visibility text.

## Repo Conventions And Pitfalls

- Preserve user changes. Check `git status --short` before edits and do not revert unrelated work.
- Prefer focused changes that match nearby patterns over broad refactors.
- For Windows-facing fixes, bump package/app versions so testers do not reinstall a stale same-version bundle. Update the workspace version, Tauri version/title, UI package versions, and UI build markers together.
- When editing [Cargo.lock](Cargo.lock), change only this workspace package entry for release bumps. Do not blanket-replace dependency versions that happen to match.
- Windows cross-builds from WSL/Linux should use `tools/build-windows-xwin.sh --ci`; bare MSVC builds can fail on missing `lib.exe` or CRT headers.
- When invoking PowerShell from bash, escape `$` variables so bash does not expand them first.
- Copy NSIS installers to a Windows local path before silent install testing; installing directly from `\\wsl.localhost` can hang.
- Windows transparency depends on WebView2 background configuration, a single intended CSS alpha layer, and xterm transparency-compatible rendering.
- SFTP sudo mode is intentionally passwordless only: it uses `sudo -n` and should fail fast when a password prompt would be required.
- Color scheme swatches should key by scheme plus index, not by color value; palettes can contain duplicate colors.

## Validation Expectations

Scale validation to the risk of the change. For UI changes, at least run `cd apps/ui && npm run check` or `npm run build`. For backend/RPC changes, run `cargo check --features desktop` and relevant tests. For release or Windows-facing changes, build packages and smoke-test the installed Windows app when possible.

Do not commit or create branches unless the user explicitly asks.
