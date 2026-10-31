# Sync Protocol

## Goals

- Multi-device convergence of user config (connections, appearance, shortcuts,
  plugin config, optional credentials) within minutes.
- End-to-end encryption: the storage backend never sees plaintext.
- Self-hosted only (WebDAV + Git). No default server URL.
- Incremental transfer: quiet cycles download nothing.

## Protocol versions

| Version | Layout | Transport |
| ------- | ------ | --------- |
| **1** (legacy) | `<group>/<record-uuid>.bin` per record | Full `list` + `get` every cycle; Git often one commit per put |
| **2** (current) | Append-only oplog under `v2/` | List ops dirs, `get` only `seq > cursor`; one Git commit per cycle |

Configure / re-key always installs a protocol-2 [`OplogDriver`](../src-tauri/src/sync/oplog.rs).
Legacy v1 blobs remain readable until purged via Settings → Config Sync →
**Purge legacy v1 data** (`sync.purgeV1`).

## Protocol 2 layout

```text
v2/devices/<device_id>/ops/<seq:012>.bin   # encrypted OpBatch
v2/snapshots/<snapshot_id>.bin             # encrypted Snapshot
v2/snapshots/latest                        # plaintext snapshot uuid
```

Each device only **appends** to its own ops directory. Peers advance a
per-device cursor so a quiet cycle performs list-only traffic (no GETs).

### Op / OpBatch

```
Op {
  group, id,
  kind: Put { payload } | Delete,
  vv: VersionVector,
  updated_at_ms, writer, schema
}
OpBatch { device, seq, ops[] }   // JSON → ChaCha20-Poly1305 envelope
```

### Snapshot + watermark

When op file count or total bytes exceed thresholds (defaults: 64 files /
2 MiB), or when the user clicks **Compact now**, the writer:

1. Encrypts a `Snapshot` of selected groups plus a **watermark**
   (`device_id → max applied seq`).
2. Writes `v2/snapshots/<uuid>.bin` and updates `v2/snapshots/latest`.
3. Deletes **only its own** ops with `seq ≤ watermark[self]` (grace period
   default 7 days) so multi-writer races do not delete peers’ tails.

Cold start / empty local engine: apply latest snapshot, seed cursors from
the watermark, then pull ops with `seq > watermark[device]`.

### Cycle (per `sync.now` / auto-sync tick)

1. `export_locals` into the in-memory `SyncEngine` (bridge).
2. Git: `fetch_remote` (if configured).
3. Oplog `cycle`: optional snapshot bootstrap → pull new ops → VV merge →
   diff local vs `last_pushed` hashes → append one `OpBatch` if needed →
   maybe compact.
4. `import_locals` back into ProfileStore / settings / vault.
5. Git: single `commit_cycle` + `push_remote`.

Stats exposed to the UI: group `SyncStats` plus `_oplog`
(`bytesDownloaded`, `bytesUploaded`, `opsApplied`, `snapshotUsed`,
`compacted`). `sync.status` reports `protocol`, `cursors`, `ownSeq`,
`lastOplog`.

## Encryption

- Master password → KEK via Argon2id (m=64 MiB, t=3, p=1; cheap params in tests).
- Per-blob DEK: random 32 B, wrapped with KEK, stored alongside ciphertext.
- AEAD: ChaCha20-Poly1305, nonce = random 12 B.
- Credentials group is off by default; requires vault unlock (and optional
  keyring auto-unlock via `sync.ensureVaultUnlock`).

## Version vectors & conflicts

Unchanged from v1 semantics:

- Each device has a stable random `device_id`.
- Local write increments the device counter in the record VV.
- On apply: dominate → fast-forward; dominated → no-op; concurrent → LWW
  by `updated_at_ms` then writer id (same as `SyncEngine` merge).

## Backend contract

```rust
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError>;
    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError>;
    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError>;
    async fn delete_blob(&self, path: &str) -> Result<(), SyncError>;
}
```

Legacy `SyncBackend` (`list`/`get`/`put`/`delete` per group) remains for
migration tooling and inspector RPCs.

### WebDAV

- HTTP `PROPFIND` / `PUT` / `GET` / `DELETE` against path-addressed blobs.
- Auth: basic; credentials in OS keychain when configured that way.

### Git

- Local working clone via `git2`.
- Working-tree writes mark the backend dirty; **one** `commit_cycle` per
  sync tick (`sync(<device>): cycle`).
- Optional history squash: `GitBackend::squash_orphan` builds an orphan
  commit with the current tree and force-pushes; peers recover via the
  existing “diverged → align to remote” fetch path.
- Auth: HTTPS PAT (keyring), SSH key, or OAuth device flow.

## Selective sync

| Group       | Default |
| ----------- | ------- |
| Connections | on      |
| Appearance  | on      |
| Shortcuts   | on      |
| PluginCfg   | on      |
| Credentials | **off** |

## Migration (v1 → v2)

1. Configure / re-key creates an `OplogDriver`. If the local sled already
   holds records, an initial snapshot is written immediately.
2. First `sync.now` exports locals and appends ops (or relies on the
   snapshot for empty remotes).
3. Optional: **Purge legacy v1 data** removes `<group>/<uuid>.bin` blobs
   only; `v2/` is untouched.
4. `settings.sync.protocol` is persisted as `2`.

## Self-hosted setup hints

See `docs/sync-self-hosting.md` for Nextcloud, Apache `mod_dav`, and Gitea.
No default endpoints ship with the application.
