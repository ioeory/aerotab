# Sync Protocol

## Goals

- Multi-device convergence of user config (connections, appearance, shortcuts,
  plugin config) within minutes.
- End-to-end encryption: the storage backend never sees plaintext.
- Self-hosted only (WebDAV + Git in v2 GA). No default server URL.

## Data model

A user's sync corpus is a set of **records**, each with:

```
Record {
  id: Uuid,
  group: Group,             // Connections | Appearance | Shortcuts | PluginCfg
  payload: bytes,           // ciphertext (ChaCha20-Poly1305)
  vv: VersionVector,        // { device_id -> u64 }
  updated_at: u64,          // wall-clock, advisory only
  schema: u16,              // record schema version
}
```

Records are stored as individual blobs under
`<root>/records/<group>/<id>.rec` for both WebDAV and Git backends.

## Encryption

- Master password → KEK via Argon2id (m=64 MiB, t=3, p=1).
- Per-record DEK: random 32 B, wrapped with KEK, stored alongside ciphertext.
- AEAD: ChaCha20-Poly1305, nonce = random 12 B.
- KEK rotation: re-wrap all DEKs; ciphertext untouched.

Credentials (SSH keys, passwords) are excluded by default; opt-in upload
encrypts them with a separate **credential KEK** that requires a confirmation
prompt every load.

## Version vectors & conflicts

- Each device has a stable random `device_id` (created on first sync setup).
- Local write increments the device's counter in the record's VV.
- On pull, for each record:
  - If incoming VV dominates local → fast-forward.
  - If local dominates incoming → no-op (push later).
  - Else → **conflict**: emit a merge entry; user picks per-field, or applies
    the configured "last-writer-wins" strategy.

## Backend contract (Rust trait sketch)

```rust
#[async_trait]
pub trait SyncBackend: Send + Sync {
    async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError>;
    async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError>;
    async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError>;
    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError>;
}
```

### WebDAV
- HTTP `PROPFIND`/`PUT`/`GET`/`DELETE`.
- Atomicity: write to `<id>.rec.tmp`, then `MOVE` to `<id>.rec`.
- Auth: basic / bearer; credentials in OS keychain.

### Git
- Local working clone managed by `git2`.
- Each sync = `fetch` → rebase local changes → write records → `commit` → `push`.
- Commit message: `sync(<device-id>): <n> records`.
- `.gitignore` excludes any non-record artifacts.
- Auth: HTTPS PAT or SSH key (reuses host key agent).

## Selective sync

User can toggle per-group sync from the settings UI:

| Group       | Default |
| ----------- | ------- |
| Connections | on      |
| Appearance  | on      |
| Shortcuts   | on      |
| PluginCfg   | off     |
| Credentials | off     |

## Migration

The importer for Tabby v1's `config.yaml` produces records in v2's schema,
each tagged with a synthetic device_id `imported-v1` so they participate in
sync immediately.

## Self-hosted setup hints

Documented in `docs/sync-self-hosting.md` (added in W12) for:

- Nextcloud (WebDAV)
- Apache `mod_dav` (WebDAV)
- Gitea (Git over HTTPS or SSH)

No default endpoints ship with the application.
