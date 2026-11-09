# Sync Protocol

## Goals

- Multi-device convergence of user config (connections, appearance, shortcuts,
  plugin config, opt-in Notes and credentials).
- End-to-end encryption of record / operation / snapshot payloads. Backends can still observe object paths, sizes, and traffic; protocol-2 snapshot pointers are plaintext.
- User-configured Git, WebDAV, S3-compatible storage, or Google Drive appDataFolder. No default self-hosted server URL or bundled cloud-account credentials.
- Incremental operation transfer with per-group / per-device progress. Listing, snapshot discovery, and peer metadata can still require traffic on quiet cycles.

## Protocol versions

| Version | Layout | Transport |
| ------- | ------ | --------- |
| **1** (legacy) | `<group>/<record-uuid>.bin` per record | Full `list` + `get` every cycle; Git often one commit per put |
| **2** (compatible) | Append-only oplog under `v2/` | Ops, snapshots, and shared latest pointer; one Git commit per cycle |
| **3** (new setup default) | Isolated oplog / immutable publications under `v3/` | Per-group coverage, peer acknowledgements, explicit retirement / safe pruning |

Configure installs an [`OplogDriver`](../src-tauri/src/sync/oplog.rs) for the requested protocol (default **3** at the RPC layer). Saved configurations without a protocol are treated as **2**, not silently upgraded. Existing protocol-2 state requires the explicit migration preview / confirmation flow before switching to 3. Legacy v1 blobs remain available to migration / purge tooling.

## Protocol 3 (0.3.0)

Protocol 3 uses separate `v3/devices/<device_id>/ops/`, `v3/snapshots/<writer>/<uuid>.bin`, and `v3/peers/<writer>/<uuid>.bin` namespaces. Snapshots are immutable encrypted publications with group-specific coverage; unlike protocol 2, there is no shared mutable `latest` pointer. Peers publish acknowledgements and retirement events. Compaction / pruning considers active peers and retained snapshot generations; an offline unacknowledged peer blocks unsafe pruning until explicitly retired. A retired device must explicitly rejoin with a new identity.

New setup tests the target, configures the engine, previews the first merge, and requires approval before first sync / auto-sync. The approval is target-bound and invalidated when relevant target / selection state changes; it is not a blanket permission to replace remote data. `sync.preview` evaluates a copy rather than applying the preview to live local stores. Git previews use an isolated preview working copy.

Maintenance RPCs include `sync.migrationPreview` / `sync.migrateV3`, device inspection / retirement / rejoin, and encrypted local recovery inspection / export / explicit restore. Migration and recovery are confirmed maintenance operations, not automatic consequences of pressing Sync now. Local journals and recovery bundles protect interrupted projection / maintenance; they do not replace an independent backup.

The protocol-2 layout and behavior below remain relevant to existing targets; do not apply its shared-pointer pruning model to protocol 3.

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

### S3-compatible object storage

- HTTPS origin endpoint, region, existing bucket, prefix, and path-style / virtual-hosted addressing. Requests use AWS Signature V4 with access key, secret key, and optional session token from the OS keyring (`sync.s3.credentials`).
- Required access is bucket listing plus object Get / Put / Delete within the configured prefix. Prefer a dedicated bucket / prefix and narrowly scoped credentials; the backend does not provision buckets or IAM policies.
- Compatibility depends on the provider's implementation of the required S3 operations; a preset or mock test is not a claim that every S3-compatible service has been live-tested.

### Google Drive

- Uses **appDataFolder**, not arbitrary My Drive / shared-drive folders. Logical paths are stored as application properties; the prefix is an application namespace, not a Drive directory picker.
- Requires a user-supplied OAuth client ID and browser-based authorization with loopback callback / PKCE. Refresh credentials remain in the OS keyring. Reauthorization isolates account identity and invalidates the current engine / approval.
- Other cloud-drive APIs (Dropbox, OneDrive) and Azure Blob are not implemented. Read-only provider tests establish connectivity / listing only, not a complete encrypted write / merge round-trip.

## Notes projection

Notes are an explicitly selected group containing ordinary notes and folders, including tags and profile associations, rather than one opaque settings bundle. Deletions propagate as tombstones. Concurrent / conflicting note edits can preserve conflict copies. Dirty frontend drafts pause Notes sync / preview; save drafts before retrying. Notes can contain sensitive text, so opt-in sync and MCP note access should be granted deliberately; this is not a Vault secret API.

## Selective sync

| Group       | Default |
| ----------- | ------- |
| Connections | on      |
| Appearance  | on      |
| Shortcuts   | on      |
| PluginCfg   | on      |
| Credentials | **off** |
| Notes       | **off** |

## Legacy migration (v1 → v2)

The following describes the legacy protocol-2 flow only. New 0.3.0 setup defaults to protocol 3; use its explicit migration / confirmation UI for existing remotes.

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
