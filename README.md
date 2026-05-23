# Tabby v2 (Refactor)

A performance-focused, full rewrite of [Tabby](https://github.com/Eugeny/tabby) in **Rust + Tauri**, preserving the SSH/serial/terminal experience while drastically reducing startup time and memory footprint.

> Status: **Phase 1 – Project Foundation (W1-W2)**. Not usable yet.

## Goals

- Cross-platform SSH/serial/terminal client (Windows, macOS, Linux).
- Native config & session sync (WebDAV + Git, self-hosted only) as a first-class feature.
- Legacy plugin compatibility for the curated priority list via a Node RPC bridge.

## Non-goals (v2 GA)

- Large-scale UI redesign.
- Re-implementing the VT parser (we reuse a mature web terminal renderer).
- Official hosted sync service.

## Performance KPIs (vs Electron Tabby baseline)

| Metric                       | Target            |
| ---------------------------- | ----------------- |
| Cold startup                 | ≤ 350 ms (P50)    |
| Idle memory (5 tabs, 30 min) | ≤ 130 MB          |
| SSH connect (key auth)       | ≤ 700 ms          |
| Terminal throughput          | 500 lines/s @60fps|
| Installer size               | < 150 MB          |

See [docs/perf-benchmark.md](docs/perf-benchmark.md) for measurement methodology.

## Layout

```
.
├── apps/ui/            # Frontend shell (Tauri webview)
├── src-tauri/          # Rust backend (Tauri host + core)
│   └── src/
│       ├── core/       # session/tab/pane lifecycle, event bus
│       ├── ipc/        # JSON-RPC protocol layer
│       ├── ssh/        # russh-based SSH client
│       ├── terminal/   # PTY I/O bridge
│       ├── serial/     # serial port channel
│       ├── sync/       # config/session sync (WebDAV, Git)
│       └── plugins/    # legacy plugin RPC bridge
├── docs/               # architecture, perf, sync protocol
└── .github/workflows/  # CI for win/macos/linux
```

## Migration scope

See [docs/architecture.md](docs/architecture.md) and the session plan for the curated priority plugin list and sync backend decisions.

## License

To be decided (likely MIT, matching upstream).
