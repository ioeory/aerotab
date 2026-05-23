# Performance Benchmark Spec

## KPIs

| Metric                       | Target            | Pass criteria                   |
| ---------------------------- | ----------------- | ------------------------------- |
| Cold startup (P50)           | ≤ 350 ms          | ± 50 ms variance across 30 runs |
| Cold startup (P95)           | ≤ 600 ms          |                                 |
| Idle memory (5 tabs, 30 min) | ≤ 130 MB RSS      | No upward drift                 |
| SSH connect (key auth)       | ≤ 700 ms          | Localhost loopback              |
| Terminal throughput          | ≥ 500 lines/s     | xterm corpus, sustained 60fps   |
| Installer size               | < 150 MB          | Signed package, each platform   |
| Plugin load time             | < 100 ms / plugin | Per priority plugin             |

## Measurement methodology

### Cold startup
- Drop OS caches between runs.
- Wall-clock from process spawn to "first paintable frame" event emitted
  by the frontend.
- 30 runs per platform; report P50 / P95.

### Idle memory
- Open 5 tabs (1 local shell + 4 idle SSH to a loopback sshd).
- Sample RSS every 30s for 30 min.
- Pass requires linear-regression slope ≤ 0.5 MB/min.

### SSH connect
- `russh` key auth against local sshd on the same host.
- Measure from "connect requested" RPC to "channel ready" event.

### Terminal throughput
- Driver replays a fixed ANSI corpus (~5000 sequences) at increasing rates.
- Pass = sustained interactive responsiveness at 500 lines/s for 60 s.

### Installer size
- Measure final signed artifact (`.exe` NSIS, `.dmg`, `.AppImage`).

## Baseline corpus

- Upstream Tabby latest stable, same machine, same workload.
- Numbers stored under `docs/baselines/<platform>-<date>.json`.

## Tooling

- Bench harness lives in `tools/bench/` (added in W1).
- CI publishes per-PR delta against the last main baseline.
