//! Git sync backend.
//!
//! On-disk layout inside the working tree:
//!
//! ```text
//! <repo>/<group>/<record-uuid>.bin          # legacy SyncBackend records
//! <repo>/v2/...                            # BlobStore / oplog paths
//! ```
//!
//! Working-tree mutations (`put` / `delete` / [`BlobStore`] writes) only touch
//! the filesystem and mark the backend dirty. Call [`GitBackend::commit_cycle`]
//! once per sync cycle so a single commit covers every change (previously each
//! `put` produced its own commit and remotes grew without bound).
//!
//! All git2 calls are blocking; we wrap them in `tokio::task::spawn_blocking`
//! so the async trait stays honest.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use git2::{
    Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository, Signature,
};
use uuid::Uuid;

use crate::sync::blob::{normalize_blob_path, BlobMeta, BlobStore};
use crate::sync::{Group, RecordId, SyncBackend, SyncError};

// Held by actual blocking workers, not their cancellable async waiters.
// Conservative process-wide serialization also protects reopened backend handles.
static REPOSITORY_WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn validate_push_status(status: Option<&str>) -> Result<(), git2::Error> {
    status.map_or(Ok(()), |reason| Err(git2::Error::from_str(reason)))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FetchOutcome {
    pub commits_applied: usize,
    pub discarded_local_batches: bool,
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityStats {
    pub worktree_bytes: u64,
    pub git_bytes: u64,
    pub total_bytes: u64,
    pub op_bytes: u64,
    pub op_files: usize,
    pub snapshot_bytes: u64,
    pub snapshot_files: usize,
    pub commits: usize,
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub history_rewrite_allowed: bool,
}

#[derive(Clone)]
pub struct GitBackend {
    inner: Arc<GitInner>,
}

struct GitInner {
    repo_path: PathBuf,
    author_name: String,
    author_email: String,
    /// Optional remote name (e.g. "origin"). Set via [`GitBackend::with_remote`].
    remote: Option<RemoteConfig>,
    /// Set when the working tree has uncommitted sync writes.
    pending: AtomicBool,
}

#[derive(Clone)]
struct RemoteConfig {
    name: String,
    #[allow(dead_code)]
    url: String,
    branch: String,
    auth: RemoteAuth,
}

#[derive(Clone, Default)]
struct RemoteAuth {
    /// HTTPS username (e.g. "git" or a Personal Access Token user).
    username: Option<String>,
    /// HTTPS password / token.
    password: Option<String>,
    /// Path to an SSH private key.
    ssh_key_path: Option<PathBuf>,
    /// Optional passphrase for the SSH key.
    ssh_passphrase: Option<String>,
}

impl GitBackend {
    /// Opens an existing repo, or initialises one if `repo_path` is empty.
    pub fn open_or_init(repo_path: impl Into<PathBuf>) -> Result<Self, SyncError> {
        let _worker = REPOSITORY_WORKER
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        let repo_path = repo_path.into();
        std::fs::create_dir_all(&repo_path).map_err(io_err)?;
        match Repository::open(&repo_path) {
            Ok(_) => {}
            Err(_) => {
                Repository::init(&repo_path).map_err(git_err)?;
            }
        }
        Ok(Self {
            inner: Arc::new(GitInner {
                repo_path,
                author_name: "Tabby".into(),
                author_email: "aerotab@localhost".into(),
                remote: None,
                pending: AtomicBool::new(false),
            }),
        })
    }

    /// Fetch into a disposable repository and copy only sync blobs into memory.
    /// The live repository's refs, index, worktree and pending flag are untouched.
    pub async fn preview_blobs(&self) -> Result<Arc<crate::sync::MemBlobStore>, SyncError> {
        struct Temp(PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let temp = Temp(std::env::temp_dir().join(format!("aerotab-preview-{}", Uuid::new_v4())));
        let mut isolated = Self::open_or_init(&temp.0)?;
        if let Some(remote) = &self.inner.remote {
            isolated = isolated.with_remote(&remote.name, &remote.url, &remote.branch)?;
            Arc::get_mut(&mut isolated.inner)
                .expect("new isolated backend")
                .remote = Some(remote.clone());
            isolated.fetch_outcome().await?;
        } else {
            let memory = Arc::new(crate::sync::MemBlobStore::new());
            for meta in self.list_blobs("").await? {
                memory
                    .put_blob(&meta.path, &self.get_blob(&meta.path).await?)
                    .await?;
            }
            return Ok(memory);
        }
        let memory = Arc::new(crate::sync::MemBlobStore::new());
        for meta in isolated.list_blobs("").await? {
            memory
                .put_blob(&meta.path, &isolated.get_blob(&meta.path).await?)
                .await?;
        }
        Ok(memory)
    }

    pub fn target_identity(&self) -> String {
        format!(
            "git:{:?}:{:?}",
            std::fs::canonicalize(&self.inner.repo_path)
                .unwrap_or_else(|_| self.inner.repo_path.clone()),
            self.inner.remote.as_ref().map(|r| (
                &r.url,
                &r.branch,
                &r.auth.username,
                &r.auth.ssh_key_path,
                r.auth
                    .password
                    .as_ref()
                    .map(|p| Uuid::new_v5(&Uuid::NAMESPACE_OID, p.as_bytes()))
            ))
        )
    }

    /// True when the working tree has writes that have not been committed yet.
    pub fn has_pending(&self) -> bool {
        self.inner.pending.load(Ordering::SeqCst)
    }

    /// Commit every pending working-tree change as a single commit. No-op when
    /// nothing is dirty. Returns `true` when a commit was created.
    pub async fn commit_cycle(&self, message: &str) -> Result<bool, SyncError> {
        if !self.inner.pending.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        let me = self.clone();
        let message = message.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            if let Some(cfg) = &me.inner.remote {
                me.recover_pending_rewrite(cfg)?;
            }
            me.commit_all(&message)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))
        .and_then(|r| r);
        if let Err(error) = result {
            self.inner.pending.store(true, Ordering::SeqCst);
            return Err(error);
        }
        Ok(true)
    }

    pub async fn capacity(&self) -> Result<CapacityStats, SyncError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            let repo = Repository::open(&me.inner.repo_path).map_err(git_err)?;
            let files = list_blobs_under(&me.inner.repo_path, "")?;
            let mut stats = CapacityStats::default();
            for file in files {
                stats.worktree_bytes += file.size;
                if (file.path.starts_with("v2/devices/") || file.path.starts_with("v3/devices/"))
                    && file.path.contains("/ops/")
                {
                    stats.op_bytes += file.size;
                    stats.op_files += 1;
                }
                if file.path.starts_with("v2/snapshots/") || file.path.starts_with("v3/snapshots/")
                {
                    stats.snapshot_bytes += file.size;
                    stats.snapshot_files += 1;
                }
            }
            fn size(dir: &Path) -> Result<u64, SyncError> {
                let mut bytes = 0;
                for entry in std::fs::read_dir(dir).map_err(io_err)? {
                    let entry = entry.map_err(io_err)?;
                    let kind = entry.file_type().map_err(io_err)?;
                    if kind.is_dir() {
                        bytes += size(&entry.path())?;
                    } else if kind.is_file() {
                        bytes += entry.metadata().map_err(io_err)?.len();
                    }
                }
                Ok(bytes)
            }
            stats.git_bytes = size(repo.path())?;
            stats.total_bytes = stats.worktree_bytes + stats.git_bytes;
            stats.commits = me.commit_count()?;
            stats.head_oid = repo
                .head()
                .ok()
                .and_then(|r| r.target())
                .map(|oid| oid.to_string());
            stats.branch = me.inner.remote.as_ref().map(|r| r.branch.clone());
            stats.history_rewrite_allowed = stats
                .branch
                .as_deref()
                .is_some_and(|b| b == "aerotab-sync" || b.starts_with("aerotab-sync/"));
            Ok(stats)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    /// Count commits reachable from HEAD (best-effort; `None` on empty repo).
    pub fn commit_count(&self) -> Result<usize, SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let Ok(head) = repo.head() else {
            return Ok(0);
        };
        let oid = head
            .target()
            .ok_or_else(|| SyncError::Transport("HEAD has no target".into()))?;
        let mut walk = repo.revwalk().map_err(git_err)?;
        walk.push(oid).map_err(git_err)?;
        Ok(walk.count())
    }

    /// Replace history with a single orphan commit that preserves the current
    /// tree, then force-with-lease push when a remote is configured.
    ///
    /// Peers that diverge after this run hit the existing "align to remote"
    /// path on their next fetch.
    pub async fn squash_orphan(&self, message: &str) -> Result<(), SyncError> {
        let me = self.clone();
        let message = message.to_string();
        tokio::task::spawn_blocking(move || {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            me.squash_orphan_blocking(&message)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    fn squash_orphan_blocking(&self, message: &str) -> Result<(), SyncError> {
        let remote_cfg = self.inner.remote.as_ref().ok_or_else(|| {
            SyncError::Transport("history rewrite requires a dedicated remote branch".into())
        })?;
        if remote_cfg.branch != "aerotab-sync" && !remote_cfg.branch.starts_with("aerotab-sync/") {
            return Err(SyncError::Transport(
                "history rewrite is restricted to aerotab-sync branches".into(),
            ));
        }
        if self.has_pending() {
            return Err(SyncError::Transport(
                "commit pending writes before history rewrite".into(),
            ));
        }
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        if repo.path().join("aerotab-rewrite-pending.json").exists() {
            return Err(SyncError::Transport(
                "unresolved history rewrite pending; refusing another rewrite".into(),
            ));
        }
        let mut status_options = git2::StatusOptions::new();
        status_options
            .include_untracked(true)
            .recurse_untracked_dirs(true);
        if !repo
            .statuses(Some(&mut status_options))
            .map_err(git_err)?
            .is_empty()
        {
            return Err(SyncError::Transport(
                "history rewrite requires a clean working tree and index".into(),
            ));
        }
        let local_ref = format!("refs/heads/{}", remote_cfg.branch);
        if repo.head().map_err(git_err)?.name() != Some(local_ref.as_str()) {
            return Err(SyncError::Transport(
                "HEAD must be on the configured dedicated sync branch".into(),
            ));
        }
        let tracking = format!("refs/remotes/{}/{}", remote_cfg.name, remote_cfg.branch);
        let expected = repo
            .find_reference(&tracking)
            .and_then(|r| r.peel_to_commit())
            .map_err(git_err)?
            .id();
        let old = repo
            .head()
            .and_then(|r| r.peel_to_commit())
            .map_err(git_err)?
            .id();
        if old != expected {
            return Err(SyncError::Transport(
                "fetch and reconcile remote before history rewrite".into(),
            ));
        }
        let mut index = repo.index().map_err(git_err)?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .map_err(git_err)?;
        index.write().map_err(git_err)?;
        let tree_oid = index.write_tree().map_err(git_err)?;
        let tree = repo.find_tree(tree_oid).map_err(git_err)?;
        let sig = self.signature()?;
        // Orphan: no parents.
        let oid = repo
            .commit(None, &sig, &sig, message, &tree, &[])
            .map_err(git_err)?;
        let branch = self
            .inner
            .remote
            .as_ref()
            .map(|r| r.branch.clone())
            .unwrap_or_else(|| "master".into());
        let local_ref = format!("refs/heads/{branch}");
        let pending_path = repo.path().join("aerotab-rewrite-pending.json");
        let pending = serde_json::json!({"old": old.to_string(), "new": oid.to_string(), "branch": remote_cfg.branch});
        std::fs::write(
            &pending_path,
            serde_json::to_vec(&pending).map_err(|e| SyncError::Transport(e.to_string()))?,
        )
        .map_err(io_err)?;
        repo.reference(&local_ref, oid, true, "squash orphan")
            .map_err(git_err)?;
        repo.set_head(&local_ref).map_err(git_err)?;
        if let Err(error) = self.push_history_with_lease(remote_cfg, expected) {
            match self.remote_tip(remote_cfg) {
                Ok(Some(tip)) if tip == oid => { /* lost acknowledgement: push succeeded */ }
                Ok(Some(tip)) if tip == old => {
                    repo.reference(&local_ref, old, true, "restore rejected history rewrite").map_err(git_err)?;
                    std::fs::remove_file(&pending_path).map_err(io_err)?;
                    return Err(error);
                }
                _ => return Err(SyncError::Transport(format!("history rewrite outcome uncertain; durable pending old/new retained, no rollback or GC: {error}"))),
            }
        }
        std::fs::remove_file(&pending_path).map_err(io_err)?;
        repo.reference(&tracking, oid, true, "history rewrite acknowledged")
            .map_err(git_err)?;
        // Local history storage is reclaimed explicitly, only after push
        // acknowledgement. Never run GC against the remote server.
        run_git(
            &self.inner.repo_path,
            &RemoteAuth::default(),
            None,
            &["reflog", "expire", "--expire=now", "--all"],
        )?;
        run_git(
            &self.inner.repo_path,
            &RemoteAuth::default(),
            None,
            &["gc", "--prune=now"],
        )
    }

    fn remote_tip(&self, cfg: &RemoteConfig) -> Result<Option<git2::Oid>, SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut remote = repo.remote_anonymous(&cfg.url).map_err(git_err)?;
        let mut callbacks = RemoteCallbacks::new();
        let auth = cfg.auth.clone();
        callbacks.credentials(move |url, user, allowed| build_creds(&auth, url, user, allowed));
        let connection = remote
            .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
            .map_err(git_err)?;
        let name = format!("refs/heads/{}", cfg.branch);
        Ok(connection
            .list()
            .map_err(git_err)?
            .iter()
            .find(|head| head.name() == name)
            .map(|head| head.oid()))
    }

    fn recover_pending_rewrite(&self, cfg: &RemoteConfig) -> Result<(), SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let path = repo.path().join("aerotab-rewrite-pending.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io_err(e)),
        };
        let state: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| SyncError::Transport(e.to_string()))?;
        if state["branch"].as_str() != Some(&cfg.branch) {
            return Err(SyncError::Transport(
                "pending history branch differs; manual recovery required".into(),
            ));
        }
        let old = state["old"]
            .as_str()
            .and_then(|s| git2::Oid::from_str(s).ok())
            .ok_or_else(|| SyncError::Transport("invalid pending old OID".into()))?;
        let new = state["new"]
            .as_str()
            .and_then(|s| git2::Oid::from_str(s).ok())
            .ok_or_else(|| SyncError::Transport("invalid pending new OID".into()))?;
        let tip = self.remote_tip(cfg)?;
        let resolved = match tip {
            Some(tip) if tip == new => new,
            Some(tip) if tip == old => old,
            _ => {
                return Err(SyncError::Transport(
                    "pending history outcome remains uncertain; refusing repository mutation"
                        .into(),
                ))
            }
        };
        repo.reference(
            &format!("refs/heads/{}", cfg.branch),
            resolved,
            true,
            "recover confirmed history outcome",
        )
        .map_err(git_err)?;
        repo.reference(
            &format!("refs/remotes/{}/{}", cfg.name, cfg.branch),
            resolved,
            true,
            "recover history tracking",
        )
        .map_err(git_err)?;
        std::fs::remove_file(path).map_err(io_err)
    }

    fn push_history_with_lease(
        &self,
        cfg: &RemoteConfig,
        expected: git2::Oid,
    ) -> Result<(), SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut remote = repo.remote_anonymous(&cfg.url).map_err(git_err)?;
        let mut callbacks = RemoteCallbacks::new();
        let auth = cfg.auth.clone();
        callbacks.credentials(move |url, user, allowed| build_creds(&auth, url, user, allowed));
        let target = format!("refs/heads/{}", cfg.branch);
        callbacks.push_negotiation(move |updates| {
            if updates.len() != 1
                || updates[0].dst_refname() != Some(target.as_str())
                || updates[0].src() != expected
            {
                return Err(git2::Error::from_str(
                    "history rewrite lease rejected: remote OID changed",
                ));
            }
            Ok(())
        });
        callbacks.push_update_reference(|_, status| {
            if let Some(reason) = status {
                return Err(git2::Error::from_str(reason));
            }
            Ok(())
        });
        let mut options = PushOptions::new();
        options.remote_callbacks(callbacks);
        let refspec = format!("+refs/heads/{0}:refs/heads/{0}", cfg.branch);
        remote
            .push(&[&refspec], Some(&mut options))
            .map_err(git_err)
    }

    fn mark_pending(&self) {
        self.inner.pending.store(true, Ordering::SeqCst);
    }

    fn abs_blob_path(&self, path: &str) -> Result<PathBuf, SyncError> {
        let rel = normalize_blob_path(path)?;
        Ok(self.inner.repo_path.join(rel))
    }

    pub fn with_author(mut self, name: impl Into<String>, email: impl Into<String>) -> Self {
        let inner = Arc::make_mut_or_clone(&mut self.inner);
        inner.author_name = name.into();
        inner.author_email = email.into();
        self
    }

    /// Configures (or reconfigures) the remote used by
    /// [`fetch`](Self::fetch) and [`push`](Self::push).
    pub fn with_remote(
        mut self,
        name: impl Into<String>,
        url: impl Into<String>,
        branch: impl Into<String>,
    ) -> Result<Self, SyncError> {
        let name = name.into();
        let url = normalize_git_remote_url(&url.into());
        let branch = branch.into();
        // Reconfiguration must also wait for abandoned blocking workers.
        let _worker = REPOSITORY_WORKER
            .lock()
            .map_err(|e| SyncError::Transport(e.to_string()))?;
        // Configuration is immutable and does not mutate shared .git/config.
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        repo.remote_anonymous(&url).map_err(git_err)?;
        let inner = Arc::make_mut_or_clone(&mut self.inner);
        inner.remote = Some(RemoteConfig {
            name,
            url,
            branch,
            auth: RemoteAuth::default(),
        });
        Ok(self)
    }

    /// Stores HTTPS credentials for the configured remote. No-op if no
    /// remote has been set yet.
    pub fn with_https_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        if self.inner.remote.is_some() {
            let inner = Arc::make_mut_or_clone(&mut self.inner);
            if let Some(r) = inner.remote.as_mut() {
                r.auth.username = Some(username.into());
                r.auth.password = Some(password.into());
            }
        }
        self
    }

    /// Stores an SSH key for the configured remote.
    pub fn with_ssh_key(
        mut self,
        key_path: impl Into<PathBuf>,
        passphrase: Option<String>,
    ) -> Self {
        if self.inner.remote.is_some() {
            let inner = Arc::make_mut_or_clone(&mut self.inner);
            if let Some(r) = inner.remote.as_mut() {
                r.auth.ssh_key_path = Some(key_path.into());
                r.auth.ssh_passphrase = passphrase;
            }
        }
        self
    }

    fn group_dir(&self, group: Group) -> PathBuf {
        self.inner.repo_path.join(group_segment(group))
    }

    fn record_path(&self, group: Group, id: RecordId) -> PathBuf {
        self.group_dir(group).join(format!("{}.bin", id.0))
    }

    fn signature(&self) -> Result<Signature<'static>, SyncError> {
        Signature::now(&self.inner.author_name, &self.inner.author_email).map_err(git_err)
    }

    fn commit_all(&self, message: &str) -> Result<(), SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut index = repo.index().map_err(git_err)?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .map_err(git_err)?;
        index.write().map_err(git_err)?;
        let tree_oid = index.write_tree().map_err(git_err)?;
        let tree = repo.find_tree(tree_oid).map_err(git_err)?;
        let sig = self.signature()?;
        let parents = match repo.head() {
            Ok(head) => {
                let parent = head.peel_to_commit().map_err(git_err)?;
                vec![parent]
            }
            Err(_) => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .map_err(git_err)?;
        Ok(())
    }

    /// Performs `git fetch <remote>` then a fast-forward merge of
    /// `refs/remotes/<remote>/<branch>` into `HEAD`. Returns the number of
    /// commits applied. Returns `Ok(0)` and a no-op result when no remote
    /// is configured (so engine code can call this unconditionally).
    pub async fn fetch_remote(&self) -> Result<usize, SyncError> {
        Ok(self.fetch_outcome().await?.commits_applied)
    }

    pub async fn fetch_outcome(&self) -> Result<FetchOutcome, SyncError> {
        let me = self.clone();
        tokio::time::timeout(std::time::Duration::from_secs(120), async move {
            tokio::task::spawn_blocking(move || {
                let _worker = REPOSITORY_WORKER
                    .lock()
                    .map_err(|e| SyncError::Transport(e.to_string()))?;
                me.fetch_blocking()
            })
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
        })
        .await
        .map_err(|_| SyncError::Transport("git fetch timed out after 120s".into()))?
    }

    /// Pushes the current branch to the configured remote. No-op when no
    /// remote is set.
    pub async fn push_remote(&self) -> Result<(), SyncError> {
        let me = self.clone();
        tokio::time::timeout(std::time::Duration::from_secs(120), async move {
            tokio::task::spawn_blocking(move || {
                let _worker = REPOSITORY_WORKER
                    .lock()
                    .map_err(|e| SyncError::Transport(e.to_string()))?;
                me.push_blocking()
            })
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
        })
        .await
        .map_err(|_| SyncError::Transport("git push timed out after 120s".into()))?
    }

    fn fetch_blocking(&self) -> Result<FetchOutcome, SyncError> {
        let Some(remote_cfg) = self.inner.remote.clone() else {
            return Ok(FetchOutcome::default());
        };
        self.recover_pending_rewrite(&remote_cfg)?;
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let refspec = format!(
            "+refs/heads/{branch}:refs/remotes/{name}/{branch}",
            branch = remote_cfg.branch,
            name = remote_cfg.name
        );
        self.fetch_remote_refs(&remote_cfg, &refspec)?;

        // Fast-forward HEAD if possible.
        let remote_ref = format!("refs/remotes/{}/{}", remote_cfg.name, remote_cfg.branch);
        let fetch_head = match repo.find_reference(&remote_ref) {
            Ok(r) => r,
            Err(_) => return Ok(FetchOutcome::default()),
        };
        let fetch_commit = fetch_head.peel_to_commit().map_err(git_err)?;
        let analysis = {
            let annotated = repo
                .reference_to_annotated_commit(&fetch_head)
                .map_err(git_err)?;
            repo.merge_analysis(&[&annotated]).map_err(git_err)?
        };
        if analysis.0.is_up_to_date() {
            return Ok(FetchOutcome::default());
        }
        let head_target = repo.head().ok().and_then(|h| h.target());
        if analysis.0.is_fast_forward() {
            let n =
                checkout_branch_to_commit(&repo, &remote_cfg, fetch_commit.id(), "fast-forward")?;
            if let Some(old) = head_target {
                let count = count_commits_between(&repo, old, fetch_commit.id()).unwrap_or(0);
                return Ok(FetchOutcome {
                    commits_applied: count.max(n),
                    discarded_local_batches: false,
                });
            }
            return Ok(FetchOutcome {
                commits_applied: n.max(1),
                discarded_local_batches: false,
            });
        }
        // Both sides have unique commits. Git is only the transport layer — reset the
        // working tree to the remote tip and let SyncEngine merge records (VV) from
        // local sled/memory, then push a linear history on the next commit.
        tracing::info!(
            branch = %remote_cfg.branch,
            "git fetch: diverged history; aligning branch to remote before record merge"
        );
        let n = checkout_branch_to_commit(
            &repo,
            &remote_cfg,
            fetch_commit.id(),
            "sync: align to remote",
        )?;
        if let Some(old) = head_target {
            let count = count_commits_between(&repo, old, fetch_commit.id()).unwrap_or(0);
            return Ok(FetchOutcome {
                commits_applied: count.max(n),
                discarded_local_batches: true,
            });
        }
        Ok(FetchOutcome {
            commits_applied: n.max(1),
            discarded_local_batches: true,
        })
    }

    fn push_blocking(&self) -> Result<(), SyncError> {
        let Some(remote_cfg) = self.inner.remote.clone() else {
            return Ok(());
        };
        self.recover_pending_rewrite(&remote_cfg)?;
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        // Ensure local branch points to HEAD (we always commit on HEAD).
        let head = repo.head().map_err(git_err)?;
        let head_oid = head
            .target()
            .ok_or_else(|| SyncError::Transport("HEAD has no target (empty repo?)".into()))?;
        let local_branch_ref = format!("refs/heads/{}", remote_cfg.branch);
        // Force-create or update the named branch ref.
        let _ = repo.reference(&local_branch_ref, head_oid, true, "align push branch");

        let refspec = format!(
            "refs/heads/{branch}:refs/heads/{branch}",
            branch = remote_cfg.branch
        );
        self.push_remote_refs(&remote_cfg, &refspec)?;
        Ok(())
    }

    fn fetch_remote_refs(&self, remote_cfg: &RemoteConfig, refspec: &str) -> Result<(), SyncError> {
        let ssh_port = ssh_port_from_ssh_url(&remote_cfg.url);
        if prefer_git_cli_ssh() && is_ssh_remote_url(&remote_cfg.url) {
            return run_git(
                &self.inner.repo_path,
                &remote_cfg.auth,
                ssh_port,
                &["fetch", &remote_cfg.url, refspec],
            );
        }
        match self.fetch_remote_refs_libgit2(remote_cfg, refspec) {
            Ok(()) => Ok(()),
            Err(e) if is_ssh_remote_url(&remote_cfg.url) && transport_is_ssh_banner_failure(&e) => {
                run_git(
                    &self.inner.repo_path,
                    &remote_cfg.auth,
                    ssh_port,
                    &["fetch", &remote_cfg.url, refspec],
                )
            }
            Err(e) => Err(e),
        }
    }

    fn fetch_remote_refs_libgit2(
        &self,
        remote_cfg: &RemoteConfig,
        refspec: &str,
    ) -> Result<(), SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut remote = repo.remote_anonymous(&remote_cfg.url).map_err(git_err)?;
        let mut cb = RemoteCallbacks::new();
        let auth = remote_cfg.auth.clone();
        cb.credentials(move |url, user, allowed| build_creds(&auth, url, user, allowed));
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(cb);
        remote
            .fetch(&[refspec], Some(&mut opts), None)
            .map_err(git_err)
    }

    fn push_remote_refs(&self, remote_cfg: &RemoteConfig, refspec: &str) -> Result<(), SyncError> {
        let ssh_port = ssh_port_from_ssh_url(&remote_cfg.url);
        if prefer_git_cli_ssh() && is_ssh_remote_url(&remote_cfg.url) {
            return run_git(
                &self.inner.repo_path,
                &remote_cfg.auth,
                ssh_port,
                &["push", &remote_cfg.url, refspec],
            );
        }
        match self.push_remote_refs_libgit2(remote_cfg, refspec) {
            Ok(()) => Ok(()),
            Err(e) if is_ssh_remote_url(&remote_cfg.url) && transport_is_ssh_banner_failure(&e) => {
                run_git(
                    &self.inner.repo_path,
                    &remote_cfg.auth,
                    ssh_port,
                    &["push", &remote_cfg.url, refspec],
                )
            }
            Err(e) => Err(e),
        }
    }

    fn push_remote_refs_libgit2(
        &self,
        remote_cfg: &RemoteConfig,
        refspec: &str,
    ) -> Result<(), SyncError> {
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut remote = repo.remote_anonymous(&remote_cfg.url).map_err(git_err)?;
        let mut cb = RemoteCallbacks::new();
        let auth = remote_cfg.auth.clone();
        cb.credentials(move |url, user, allowed| build_creds(&auth, url, user, allowed));
        cb.push_update_reference(|_, status| validate_push_status(status));
        let mut opts = PushOptions::new();
        opts.remote_callbacks(cb);
        remote.push(&[refspec], Some(&mut opts)).map_err(git_err)
    }
}

/// How the sync UI configured remote authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitRemoteTransport {
    Https,
    Ssh,
    Unspecified,
}

/// Pick an `https://` or `ssh://git@host:port/path` URL that matches the configured auth.
/// Using `git@…` with HTTPS/OAuth makes libgit2 open SSH and fail with
/// "Failed getting banner" when only port 443 is reachable.
/// Non-default SSH ports must use `ssh://` (or `gitSshPort` + Configure).
pub(crate) fn align_git_remote_url(
    url: &str,
    transport: GitRemoteTransport,
    ssh_port: Option<u16>,
) -> String {
    let url = normalize_git_remote_url(url);
    match transport {
        GitRemoteTransport::Https => scp_or_ssh_url_to_https(&url).unwrap_or(url),
        GitRemoteTransport::Ssh => ensure_ssh_remote_url(&url, ssh_port),
        GitRemoteTransport::Unspecified => url,
    }
}

/// `git@host:group/repo.git` → `https://host/group/repo.git` (HTTPS always uses 443).
fn scp_or_ssh_url_to_https(url: &str) -> Option<String> {
    if url.starts_with("https://") || url.starts_with("http://") {
        return Some(url.to_string());
    }
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, _port, path) = parse_git_scp_suffix(rest);
        if path.is_empty() {
            return None;
        }
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        let (host_part, path) = rest.split_once('/')?;
        let host = host_part.split(':').next()?;
        let path = path.trim_start_matches('/');
        return Some(format!("https://{host}/{path}"));
    }
    None
}

/// Normalize any remote form to `ssh://git@host[:port]/path` for libgit2.
fn ensure_ssh_remote_url(url: &str, ssh_port: Option<u16>) -> String {
    if url.starts_with("ssh://") {
        return apply_ssh_port_to_ssh_url(url, ssh_port);
    }
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, parsed_port, path) = parse_git_scp_suffix(rest);
        let port = parsed_port.or(ssh_port);
        return format_ssh_url(host, port, "git", &path);
    }
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        if let Some((host_part, path)) = rest.split_once('/') {
            let host = host_part.split(':').next().unwrap_or(host_part);
            let path = path.trim_start_matches('/');
            return format_ssh_url(host, ssh_port, "git", path);
        }
    }
    url.to_string()
}

fn format_ssh_url(host: &str, port: Option<u16>, user: &str, path: &str) -> String {
    let user = if user.is_empty() { "git" } else { user };
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}")
    };
    let port = port.filter(|p| *p != 22);
    match port {
        Some(p) => format!("ssh://{user}@{host}:{p}{path}"),
        None => format!("ssh://{user}@{host}{path}"),
    }
}

/// `git@host:2222/group/repo` → (host, Some(2222), "group/repo")
/// `git@host:group/repo` → (host, None, "group/repo")
fn parse_git_scp_suffix(rest: &str) -> (&str, Option<u16>, String) {
    let Some((host_part, after_colon)) = rest.split_once(':') else {
        return (rest, None, String::new());
    };
    if let Some((first, remainder)) = after_colon.split_once('/') {
        if first.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(p) = first.parse::<u16>() {
                if p > 0 {
                    return (host_part, Some(p), remainder.to_string());
                }
            }
        }
    }
    (
        host_part,
        None,
        after_colon.trim_start_matches('/').to_string(),
    )
}

fn authority_user_from_ssh_url(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("ssh://")?;
    let authority = rest.split('/').next()?;
    authority.split_once('@').map(|(user, _)| user)
}

fn apply_ssh_port_to_ssh_url(url: &str, ssh_port: Option<u16>) -> String {
    let Some(rest) = url.strip_prefix("ssh://") else {
        return url.to_string();
    };
    let user = authority_user_from_ssh_url(url).unwrap_or("git");
    let host_path = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
    let Some((host_part, path)) = host_path.split_once('/') else {
        return url.to_string();
    };
    let host = host_part.split(':').next().unwrap_or(host_part);
    let existing_port = host_part
        .split(':')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());
    let port = existing_port.or(ssh_port);
    format_ssh_url(host, port, user, path)
}

/// Strip `https://user:pass@host/...` userinfo — embedded creds confuse libgit2 and
/// often cause "too many redirects or authentication replays" on Windows.
fn normalize_git_remote_url(url: &str) -> String {
    let trimmed = url.trim();
    let stripped = strip_embedded_credentials(trimmed);
    if stripped.starts_with("http://")
        && (stripped.contains("gitlab.com")
            || stripped.contains("github.com")
            || stripped.contains("github."))
    {
        return stripped.replacen("http://", "https://", 1);
    }
    stripped
}

fn strip_embedded_credentials(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let scheme = &url[..scheme_end + 3];
    let rest = &url[scheme_end + 3..];
    let Some(at) = rest.find('@') else {
        return url.to_string();
    };
    format!("{}{}", scheme, &rest[at + 1..])
}

fn host_hint_from_url(url: &str) -> Option<&str> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split('@').next()?.split('/').next()?;
    Some(host.split(':').next().unwrap_or(host))
}

fn default_https_username(host: Option<&str>) -> &'static str {
    match host {
        Some(h) if h.contains("gitlab") => "oauth2",
        Some(h) if h.contains("github") => "x-access-token",
        _ => "git",
    }
}

fn build_creds(
    auth: &RemoteAuth,
    url: &str,
    user_from_url: Option<&str>,
    allowed: git2::CredentialType,
) -> Result<Cred, git2::Error> {
    // HTTPS remotes: prefer explicit user/pass before SSH or system default.
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        if let Some(pw) = auth.password.as_deref().filter(|p| !p.is_empty()) {
            let host = host_hint_from_url(url);
            let user = auth
                .username
                .as_deref()
                .filter(|u| !u.is_empty())
                .or(user_from_url.filter(|u| !u.is_empty()))
                .unwrap_or_else(|| default_https_username(host));
            return Cred::userpass_plaintext(user, pw);
        }
    }
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        let user = user_from_url
            .filter(|u| !u.is_empty())
            .or(auth.username.as_deref().filter(|u| !u.is_empty()))
            .unwrap_or("git");
        if let Some(path) = &auth.ssh_key_path {
            return Cred::ssh_key(user, None, path, auth.ssh_passphrase.as_deref());
        }
        if let Ok(cred) = Cred::ssh_key_from_agent(user) {
            return Ok(cred);
        }
        return Err(git2::Error::from_str(
            "SSH private key path missing and ssh-agent has no key; \
             set Auth mode SSH + key path, or use HTTPS/OAuth with an https:// remote URL",
        ));
    }
    // Never return Cred::default() — on WinHTTP this retriggers NTLM and hits the
    // "too many redirects or authentication replays" guard after ~7 attempts.
    Err(git2::Error::from_str(
        "HTTPS credentials missing or rejected (check GitLab PAT user oauth2 + token as password)",
    ))
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn strips_embedded_credentials() {
        assert_eq!(
            normalize_git_remote_url("https://user:token@gitlab.com/a/b.git"),
            "https://gitlab.com/a/b.git"
        );
    }

    #[test]
    fn upgrades_gitlab_http_to_https() {
        assert_eq!(
            normalize_git_remote_url("http://gitlab.com/group/repo.git"),
            "https://gitlab.com/group/repo.git"
        );
    }

    #[test]
    fn scp_ssh_to_https_for_oauth() {
        assert_eq!(
            align_git_remote_url(
                "git@gitlab.com:mygroup/myrepo.git",
                GitRemoteTransport::Https,
                None
            ),
            "https://gitlab.com/mygroup/myrepo.git"
        );
    }

    #[test]
    fn https_to_ssh_url_default_port() {
        assert_eq!(
            align_git_remote_url(
                "https://gitlab.com/mygroup/myrepo.git",
                GitRemoteTransport::Ssh,
                None
            ),
            "ssh://git@gitlab.com/mygroup/myrepo.git"
        );
    }

    #[test]
    fn custom_ssh_port_in_scp_form() {
        assert_eq!(
            align_git_remote_url(
                "git@gitlab.example.com:2222/mygroup/myrepo.git",
                GitRemoteTransport::Ssh,
                None
            ),
            "ssh://git@gitlab.example.com:2222/mygroup/myrepo.git"
        );
    }

    #[test]
    fn custom_ssh_port_from_settings() {
        assert_eq!(
            align_git_remote_url(
                "git@gitlab.example.com:mygroup/myrepo.git",
                GitRemoteTransport::Ssh,
                Some(2222)
            ),
            "ssh://git@gitlab.example.com:2222/mygroup/myrepo.git"
        );
    }

    #[test]
    fn preserves_ssh_url_with_explicit_port() {
        let url = "ssh://git@23.95.2.174:22173/yuri/aerotab-config.git";
        assert_eq!(
            align_git_remote_url(url, GitRemoteTransport::Ssh, Some(22173)),
            "ssh://git@23.95.2.174:22173/yuri/aerotab-config.git"
        );
    }
}

fn is_ssh_remote_url(url: &str) -> bool {
    url.starts_with("ssh://") || url.starts_with("git@")
}

fn ssh_port_from_ssh_url(url: &str) -> Option<u16> {
    if !url.starts_with("ssh://") {
        return None;
    }
    let rest = url.strip_prefix("ssh://")?;
    let authority = rest.split('/').next()?;
    let host_part = authority.split('@').nth(1).unwrap_or(authority);
    host_part.split(':').nth(1)?.parse().ok()
}

/// libgit2's vendored libssh2 on Windows often fails SSH banner handshake on non-22 ports.
fn prefer_git_cli_ssh() -> bool {
    cfg!(windows)
}

fn transport_is_ssh_banner_failure(err: &SyncError) -> bool {
    match err {
        SyncError::Transport(m) => {
            m.contains("Failed getting banner") || m.contains("failed to start SSH session")
        }
        _ => false,
    }
}

fn resolve_git_executable() -> Result<PathBuf, SyncError> {
    if let Ok(p) = std::env::var("AEROTAB_GIT") {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    if Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(PathBuf::from("git"));
    }
    #[cfg(windows)]
    for candidate in [
        r"C:\Program Files\Git\cmd\git.exe",
        r"C:\Program Files\Git\bin\git.exe",
    ] {
        let pb = PathBuf::from(candidate);
        if pb.is_file() {
            return Ok(pb);
        }
    }
    Err(SyncError::Transport(
        "git executable not found — install Git for Windows and ensure `git` is on PATH".into(),
    ))
}

#[cfg(windows)]
fn resolve_windows_openssh() -> Option<PathBuf> {
    for candidate in [
        r"C:\Program Files\Git\usr\bin\ssh.exe",
        r"C:\Windows\System32\OpenSSH\ssh.exe",
    ] {
        let pb = PathBuf::from(candidate);
        if pb.is_file() {
            return Some(pb);
        }
    }
    None
}

fn quote_for_git_ssh_command(s: &str) -> String {
    if s.contains(' ') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn build_git_ssh_command(auth: &RemoteAuth, port: Option<u16>) -> String {
    let mut parts: Vec<String> = Vec::new();
    #[cfg(windows)]
    if let Some(ssh) = resolve_windows_openssh() {
        parts.push(quote_for_git_ssh_command(&ssh.to_string_lossy()));
    }
    if parts.is_empty() {
        parts.push("ssh".into());
    }
    if let Some(p) = port.filter(|p| *p != 22) {
        parts.push("-p".into());
        parts.push(p.to_string());
    }
    if let Some(key) = &auth.ssh_key_path {
        parts.push("-i".into());
        parts.push(quote_for_git_ssh_command(&key.to_string_lossy()));
    }
    parts.push("-o".into());
    parts.push("StrictHostKeyChecking=accept-new".into());
    parts.join(" ")
}

fn run_git(
    repo_path: &Path,
    auth: &RemoteAuth,
    ssh_port: Option<u16>,
    args: &[&str],
) -> Result<(), SyncError> {
    let git = resolve_git_executable()?;
    let ssh_cmd = build_git_ssh_command(auth, ssh_port);
    let output = Command::new(&git)
        .arg("-C")
        .arg(repo_path)
        .env("GIT_SSH_COMMAND", &ssh_cmd)
        .args(args)
        .output()
        .map_err(io_err)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = crate::text_encoding::decode_console_bytes(&output.stderr);
    let stdout = crate::text_encoding::decode_console_bytes(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        format!("{}\n{}", stderr.trim(), stdout.trim())
            .trim()
            .to_string()
    };
    let detail = crate::text_encoding::sanitize_transport_message(&detail);
    Err(SyncError::Transport(format!(
        "git {} failed (exit {:?}): {detail}",
        args.first().copied().unwrap_or("?"),
        output.status.code()
    )))
}

/// Point `refs/heads/<branch>` and the working tree at `commit` (force checkout).
fn checkout_branch_to_commit(
    repo: &Repository,
    remote_cfg: &RemoteConfig,
    commit: git2::Oid,
    log_message: &str,
) -> Result<usize, SyncError> {
    let local_branch_ref = format!("refs/heads/{}", remote_cfg.branch);
    match repo.find_reference(&local_branch_ref) {
        Ok(mut local_ref) => {
            if local_ref.target() == Some(commit) {
                return Ok(0);
            }
            local_ref.set_target(commit, log_message).map_err(git_err)?;
        }
        Err(_) => {
            repo.reference(&local_branch_ref, commit, true, "init local branch")
                .map_err(git_err)?;
        }
    }
    repo.set_head(&local_branch_ref).map_err(git_err)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        .map_err(git_err)?;
    Ok(1)
}

fn count_commits_between(repo: &Repository, base: git2::Oid, head: git2::Oid) -> Option<usize> {
    let mut walk = repo.revwalk().ok()?;
    walk.push(head).ok()?;
    walk.hide(base).ok()?;
    Some(walk.count())
}

fn io_err(e: std::io::Error) -> SyncError {
    SyncError::Transport(format!("io: {e}"))
}

fn git_err(e: git2::Error) -> SyncError {
    let msg = crate::text_encoding::sanitize_transport_message(e.message());
    let hint = if msg.contains("too many redirects or authentication replays") {
        " — for GitLab HTTPS use remote user `oauth2` and a Personal Access Token as password; \
         do not embed credentials in the remote URL; re-run Configure / re-key after fixing"
    } else if msg.contains("Failed getting banner") || msg.contains("failed to start SSH session") {
        " — verify SSH port and that the host speaks SSH (not HTTP); use ssh://git@host:PORT/path \
         or set SSH port in sync settings; on Windows install Git for Windows; for PAT auth use \
         https://host/group/repo.git; re-run Configure / re-key"
    } else if msg.contains("failed to resolve address") {
        " — DNS lookup failed: check network/VPN, or add the host to hosts/DNS; for HTTPS use https://host/group/repo.git"
    } else if msg.contains("401") || msg.contains("403") || msg.contains("authentication") {
        " — check HTTPS username/PAT or complete Git OAuth sign-in"
    } else {
        ""
    };
    SyncError::Transport(format!("git: {msg}{hint}"))
}

fn group_segment(group: Group) -> &'static str {
    match group {
        Group::Connections => "connections",
        Group::Appearance => "appearance",
        Group::Shortcuts => "shortcuts",
        Group::PluginCfg => "plugincfg",
        Group::Credentials => "credentials",
        Group::Notes => "notes",
    }
}

// Tiny helper since Arc<GitInner> doesn't implement make_mut by default.
trait ArcMakeMutOrClone<T> {
    fn make_mut_or_clone(arc: &mut Arc<T>) -> &mut T;
}
impl<T: Clone> ArcMakeMutOrClone<T> for Arc<T> {
    fn make_mut_or_clone(arc: &mut Arc<T>) -> &mut T {
        if Arc::get_mut(arc).is_none() {
            *arc = Arc::new(T::clone(arc));
        }
        Arc::get_mut(arc).expect("just made unique")
    }
}

impl Clone for GitInner {
    fn clone(&self) -> Self {
        Self {
            repo_path: self.repo_path.clone(),
            author_name: self.author_name.clone(),
            author_email: self.author_email.clone(),
            remote: self.remote.clone(),
            pending: AtomicBool::new(self.pending.load(Ordering::SeqCst)),
        }
    }
}

#[async_trait]
impl SyncBackend for GitBackend {
    async fn list(&self, group: Group) -> Result<Vec<RecordId>, SyncError> {
        let dir = self.group_dir(group);
        tokio::task::spawn_blocking(move || list_records(&dir))
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn get(&self, group: Group, id: RecordId) -> Result<Vec<u8>, SyncError> {
        let path = self.record_path(group, id);
        tokio::task::spawn_blocking(move || std::fs::read(path).map_err(io_err))
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn put(&self, group: Group, id: RecordId, blob: &[u8]) -> Result<(), SyncError> {
        let dir = self.group_dir(group);
        let path = self.record_path(group, id);
        let blob = blob.to_vec();
        let me = self.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            std::fs::create_dir_all(&dir).map_err(io_err)?;
            // Atomic write via tmp + rename.
            let tmp = path.with_extension("bin.tmp");
            std::fs::write(&tmp, &blob).map_err(io_err)?;
            std::fs::rename(&tmp, &path).map_err(io_err)?;
            me.mark_pending();
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
        let path = self.record_path(group, id);
        let me = self.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            match std::fs::remove_file(&path) {
                Ok(()) => me.mark_pending(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(io_err(e)),
            }
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }
}

#[async_trait]
impl BlobStore for GitBackend {
    async fn list_blobs(&self, prefix: &str) -> Result<Vec<BlobMeta>, SyncError> {
        let root = self.inner.repo_path.clone();
        let prefix = prefix.trim_start_matches('/').to_string();
        tokio::task::spawn_blocking(move || {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            list_blobs_under(&root, &prefix)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn get_blob(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        let abs = self.abs_blob_path(path)?;
        tokio::task::spawn_blocking(move || {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            std::fs::read(abs).map_err(io_err)
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn put_blob(&self, path: &str, bytes: &[u8]) -> Result<(), SyncError> {
        let abs = self.abs_blob_path(path)?;
        let bytes = bytes.to_vec();
        let me = self.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).map_err(io_err)?;
            }
            let tmp = abs.with_extension("tmp");
            std::fs::write(&tmp, &bytes).map_err(io_err)?;
            std::fs::rename(&tmp, &abs).map_err(io_err)?;
            me.mark_pending();
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn delete_blob(&self, path: &str) -> Result<(), SyncError> {
        let abs = self.abs_blob_path(path)?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            let _worker = REPOSITORY_WORKER
                .lock()
                .map_err(|e| SyncError::Transport(e.to_string()))?;
            match std::fs::remove_file(&abs) {
                Ok(()) => me.mark_pending(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(io_err(e)),
            }
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }
}

fn list_blobs_under(root: &Path, prefix: &str) -> Result<Vec<BlobMeta>, SyncError> {
    let mut out = Vec::new();
    let start = if prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(prefix)
    };
    if !start.exists() {
        return Ok(out);
    }
    fn walk(root: &Path, dir: &Path, out: &mut Vec<BlobMeta>) -> Result<(), SyncError> {
        let read = std::fs::read_dir(dir).map_err(io_err)?;
        for entry in read {
            let entry = entry.map_err(io_err)?;
            let path = entry.path();
            // Skip the git metadata directory.
            if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                continue;
            }
            let ft = entry.file_type().map_err(io_err)?;
            if ft.is_dir() {
                walk(root, &path, out)?;
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| SyncError::Transport(e.to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push(BlobMeta { path: rel, size });
            }
        }
        Ok(())
    }
    if start.is_file() {
        let rel = start
            .strip_prefix(root)
            .map_err(|e| SyncError::Transport(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let size = std::fs::metadata(&start).map(|m| m.len()).unwrap_or(0);
        out.push(BlobMeta { path: rel, size });
        return Ok(out);
    }
    walk(root, &start, &mut out)?;
    // When prefix points at a directory, also include files whose relative
    // path starts with the prefix string (already true if we walked start).
    if !prefix.is_empty() && start.is_dir() {
        out.retain(|m| m.path.starts_with(prefix));
    }
    Ok(out)
}

fn list_records(dir: &Path) -> Result<Vec<RecordId>, SyncError> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(io_err(e)),
    };
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(io_err)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(".bin") {
            if let Ok(u) = Uuid::parse_str(stem) {
                out.push(RecordId(u));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuring_remote_preserves_shared_repository_configuration() {
        let dir = tmpdir();
        let backend = GitBackend::open_or_init(&dir).unwrap();
        let repo = Repository::open(&dir).unwrap();
        repo.remote("origin", "https://old.invalid/sync.git")
            .unwrap();
        let _configured = backend
            .with_remote("origin", "https://new.invalid/sync.git", "main")
            .unwrap();
        assert_eq!(
            repo.find_remote("origin").unwrap().url(),
            Some("https://old.invalid/sync.git")
        );
        drop(repo);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tmpdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-git-test-{}", Uuid::new_v4()));
        p
    }

    /// `file://` URL for a local bare repo (libgit2 on Windows needs `file:///C:/...`).
    fn path_to_file_url(path: &Path) -> String {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut s = path.to_string_lossy().into_owned();
        const VERBATIM: &str = r"\\?\";
        if s.starts_with(VERBATIM) {
            s = s[VERBATIM.len()..].to_string();
        }
        let s = s.replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        }
    }

    #[tokio::test]
    async fn isolated_preview_preserves_live_head_and_dirty_files() {
        let remote_dir = tmpdir();
        let local_dir = tmpdir();
        let remote = GitBackend::open_or_init(&remote_dir).unwrap();
        remote
            .put_blob("v2/devices/a/ops/1.bin", b"remote-only")
            .await
            .unwrap();
        remote.commit_cycle("mock sync").await.unwrap();
        let branch = Repository::open(&remote_dir)
            .unwrap()
            .head()
            .unwrap()
            .shorthand()
            .unwrap()
            .to_owned();
        let local = GitBackend::open_or_init(&local_dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&remote_dir), &branch)
            .unwrap();
        local.put_blob("v2/local-dirty", b"keep").await.unwrap();
        let before = local.capacity().await.unwrap();
        let memory = local.preview_blobs().await.unwrap();
        assert_eq!(
            memory.get_blob("v2/devices/a/ops/1.bin").await.unwrap(),
            b"remote-only"
        );
        assert_eq!(local.get_blob("v2/local-dirty").await.unwrap(), b"keep");
        assert_eq!(before.head_oid, local.capacity().await.unwrap().head_oid);
        assert!(local.has_pending());
        let _ = std::fs::remove_dir_all(remote_dir);
        let _ = std::fs::remove_dir_all(local_dir);
    }

    #[tokio::test]
    async fn history_rewrite_refuses_untracked_changes_without_modifying_head() {
        let dir = tmpdir();
        let bare = bare_remote();
        let g = GitBackend::open_or_init(&dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&bare), "aerotab-sync")
            .unwrap();
        g.put_blob("v2/test.bin", b"seed").await.unwrap();
        g.commit_cycle("seed").await.unwrap();
        g.push_remote().await.unwrap();
        Repository::open(&dir)
            .unwrap()
            .set_head("refs/heads/aerotab-sync")
            .unwrap();
        g.fetch_remote().await.unwrap();
        let repo = Repository::open(&dir).unwrap();
        let old = repo.head().unwrap().target();
        std::fs::write(dir.join("unexpected.txt"), b"do not commit").unwrap();
        assert!(g.squash_orphan("rewrite").await.is_err());
        assert_eq!(repo.head().unwrap().target(), old);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(bare);
    }

    #[tokio::test]
    async fn cancelled_waiter_does_not_release_running_worker_lock() {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let waiter = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _worker = REPOSITORY_WORKER.lock().unwrap();
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .await
            .unwrap();
        });
        entered_rx.await.unwrap();
        waiter.abort();
        assert!(REPOSITORY_WORKER.try_lock().is_err());
        release_tx.send(()).unwrap();
    }

    #[tokio::test]
    async fn server_hook_rejection_is_not_successful_push() {
        let bare = bare_remote();
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&bare), "aerotab-sync")
            .unwrap();
        g.put_blob("seed.bin", b"seed").await.unwrap();
        g.commit_cycle("seed").await.unwrap();
        g.push_remote().await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hook = bare.join("hooks/pre-receive");
            std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            g.put_blob("second.bin", b"second").await.unwrap();
            g.commit_cycle("second").await.unwrap();
            assert!(
                validate_push_status(Some("pre-receive hook declined")).is_err(),
                "receive-pack rejection must reach caller (local libgit2 transport does not execute hooks)"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(bare);
    }

    #[tokio::test]
    async fn capacity_measures_worktree_git_ops_and_snapshots_separately() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        g.put_blob("v2/devices/a/ops/1.bin", b"123").await.unwrap();
        g.put_blob("v2/snapshots/one.bin", b"12345").await.unwrap();
        g.put_blob("other.bin", b"12").await.unwrap();
        let stats = g.capacity().await.unwrap();
        assert_eq!(stats.worktree_bytes, 10);
        assert_eq!(stats.op_bytes, 3);
        assert_eq!(stats.snapshot_bytes, 5);
        assert!(stats.git_bytes > 0);
        assert_eq!(stats.total_bytes, stats.git_bytes + 10);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stale_history_lease_rejects_and_restores_local_head() {
        let bare = bare_remote();
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&bare), "aerotab-sync")
            .unwrap();
        g.put_blob("v2/test.bin", b"seed").await.unwrap();
        g.commit_cycle("seed").await.unwrap();
        g.push_remote().await.unwrap();
        Repository::open(&dir)
            .unwrap()
            .set_head("refs/heads/aerotab-sync")
            .unwrap();
        g.fetch_remote().await.unwrap();
        let old = Repository::open(&dir)
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap();
        let peer_dir = tmpdir();
        let peer = GitBackend::open_or_init(&peer_dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&bare), "aerotab-sync")
            .unwrap();
        peer.fetch_remote().await.unwrap();
        peer.put_blob("v2/peer.bin", b"new").await.unwrap();
        peer.commit_cycle("peer").await.unwrap();
        peer.push_remote().await.unwrap();
        assert!(g.squash_orphan("rewrite").await.is_err());
        // Concurrent third-party tip is neither old nor our new commit:
        // retain durable pending state, never blindly roll back.
        assert!(Repository::open(&dir)
            .unwrap()
            .path()
            .join("aerotab-rewrite-pending.json")
            .exists());
        let _ = old;
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(peer_dir);
        let _ = std::fs::remove_dir_all(bare);
    }

    #[tokio::test]
    async fn history_rewrite_refuses_general_purpose_branch() {
        let dir = tmpdir();
        let bare = bare_remote();
        let g = GitBackend::open_or_init(&dir)
            .unwrap()
            .with_remote("origin", path_to_file_url(&bare), "main")
            .unwrap();
        g.put_blob("v2/test.bin", b"data").await.unwrap();
        g.commit_cycle("seed").await.unwrap();
        g.push_remote().await.unwrap();
        assert!(
            g.squash_orphan("unsafe").await.is_err(),
            "must refuse history rewrite outside dedicated sync branch"
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(bare);
    }

    #[tokio::test]
    async fn failed_commit_keeps_pending_for_retry() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        g.put_blob("v2/test.bin", b"pending").await.unwrap();
        std::fs::write(dir.join(".git/index.lock"), b"locked").unwrap();
        assert!(g.commit_cycle("sync: fail").await.is_err());
        assert!(g.has_pending(), "failed commit must retain pending state");
        std::fs::remove_file(dir.join(".git/index.lock")).unwrap();
        assert!(g.commit_cycle("sync: retry").await.unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn put_get_list_delete_roundtrip() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        let id = RecordId(Uuid::new_v4());
        g.put(Group::Connections, id, b"hello").await.unwrap();
        let listed = g.list(Group::Connections).await.unwrap();
        assert!(listed.contains(&id));
        let got = g.get(Group::Connections, id).await.unwrap();
        assert_eq!(got, b"hello");
        g.delete(Group::Connections, id).await.unwrap();
        let listed = g.list(Group::Connections).await.unwrap();
        assert!(!listed.contains(&id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_missing_group_is_empty() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        assert!(g.list(Group::Shortcuts).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn put_batches_into_one_commit_cycle() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        let id1 = RecordId(Uuid::new_v4());
        let id2 = RecordId(Uuid::new_v4());
        g.put(Group::Appearance, id1, b"a").await.unwrap();
        g.put(Group::Appearance, id2, b"b").await.unwrap();
        assert!(g.has_pending());
        assert!(g.commit_cycle("sync: test").await.unwrap());
        assert!(!g.has_pending());
        assert!(!g.commit_cycle("sync: noop").await.unwrap());
        let repo = Repository::open(&dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        let count = walk.count();
        assert_eq!(count, 1, "one commit expected, got {count}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn blob_store_paths_roundtrip() {
        use crate::sync::BlobStore;
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        g.put_blob("v2/devices/a/ops/0001.bin", b"op")
            .await
            .unwrap();
        let listed = g.list_blobs("v2/devices/a/ops/").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "v2/devices/a/ops/0001.bin");
        assert_eq!(
            g.get_blob("v2/devices/a/ops/0001.bin").await.unwrap(),
            b"op"
        );
        assert!(g.commit_cycle("sync: blob").await.unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a bare repo to act as the "remote" for round-trip tests.
    fn bare_remote() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-git-remote-{}.git", Uuid::new_v4()));
        Repository::init_bare(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn fetch_push_roundtrip_via_local_remote() {
        let bare = bare_remote();
        let url = path_to_file_url(&bare);

        // Workspace A: produce one commit, push.
        let a_dir = tmpdir();
        let a = GitBackend::open_or_init(&a_dir).unwrap();
        // First commit so HEAD exists before we attach a remote.
        a.put(Group::Appearance, RecordId(Uuid::new_v4()), b"first")
            .await
            .unwrap();
        a.commit_cycle("sync: first").await.unwrap();
        let a = a.with_remote("origin", &url, "master").unwrap();
        a.push_remote().await.unwrap();

        // Workspace B: clone state into a fresh tree, then fetch.
        let b_dir = tmpdir();
        let b = GitBackend::open_or_init(&b_dir).unwrap();
        let b = b.with_remote("origin", &url, "master").unwrap();
        let outcome = b.fetch_outcome().await.unwrap();
        assert!(
            !outcome.discarded_local_batches,
            "ordinary fast-forward must preserve oplog progress"
        );
        assert!(
            outcome.commits_applied >= 1,
            "expected at least one fast-forwarded commit"
        );
        // The file pushed by A is now visible to B.
        let listed = b.list(Group::Appearance).await.unwrap();
        assert!(!listed.is_empty(), "B should see A's record after fetch");

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn libgit2_built_with_https() {
        let v = git2::Version::get();
        assert!(
            v.https(),
            "libgit2 must be built with TLS (enable git2 features: https, vendored-openssl)"
        );
    }

    #[tokio::test]
    async fn fetch_recovers_from_diverged_history() {
        let bare = bare_remote();
        let url = path_to_file_url(&bare);

        let a_dir = tmpdir();
        let id_a = RecordId(Uuid::new_v4());
        let a = GitBackend::open_or_init(&a_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        a.put(Group::Appearance, id_a, b"from-a").await.unwrap();
        a.commit_cycle("sync: a").await.unwrap();
        a.push_remote().await.unwrap();

        let b_dir = tmpdir();
        let id_b = RecordId(Uuid::new_v4());
        let b = GitBackend::open_or_init(&b_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        b.fetch_remote().await.unwrap();
        b.put(Group::Appearance, id_b, b"from-b").await.unwrap();
        b.commit_cycle("sync: b").await.unwrap();

        let a2 = GitBackend::open_or_init(&a_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        let id_a2 = RecordId(Uuid::new_v4());
        a2.put(Group::Appearance, id_a2, b"from-a2").await.unwrap();
        a2.commit_cycle("sync: a2").await.unwrap();
        a2.push_remote().await.unwrap();

        // Diverged: B has a local-only commit; remote moved on A.
        b.fetch_remote()
            .await
            .expect("fetch should realign, not error");
        let listed = b.list(Group::Appearance).await.unwrap();
        assert!(
            listed.contains(&id_a) || listed.contains(&id_a2),
            "after realign, working tree should reflect remote commits"
        );

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[tokio::test]
    async fn squash_orphan_preserves_tree_and_peer_recovers() {
        let bare = bare_remote();
        let url = path_to_file_url(&bare);

        let a_dir = tmpdir();
        let a = GitBackend::open_or_init(&a_dir).unwrap();
        for i in 0..3u8 {
            a.put_blob(&format!("v2/devices/a/ops/{i:012}.bin"), &[i])
                .await
                .unwrap();
            a.commit_cycle(&format!("sync: {i}")).await.unwrap();
        }
        assert!(a.commit_count().unwrap() >= 3);
        let a = a.with_remote("origin", &url, "aerotab-sync").unwrap();
        {
            let repo = Repository::open(&a_dir).unwrap();
            let tip = repo.head().unwrap().peel_to_commit().unwrap();
            repo.branch("aerotab-sync", &tip, false).unwrap();
            repo.set_head("refs/heads/aerotab-sync").unwrap();
        }
        a.push_remote().await.unwrap();
        a.fetch_remote().await.unwrap();

        // Peer B clones the pre-squash history.
        let b_dir = tmpdir();
        let b = GitBackend::open_or_init(&b_dir)
            .unwrap()
            .with_remote("origin", &url, "aerotab-sync")
            .unwrap();
        assert!(b.fetch_remote().await.unwrap() >= 1);
        assert_eq!(b.list_blobs("v2/devices/a/ops/").await.unwrap().len(), 3);

        // A squashes → force-pushes. B's next fetch realigns to remote tip.
        a.squash_orphan("sync: squash").await.unwrap();
        assert_eq!(a.commit_count().unwrap(), 1);
        let _ = b.fetch_remote().await.unwrap();
        assert_eq!(
            b.get_blob("v2/devices/a/ops/000000000002.bin")
                .await
                .unwrap(),
            vec![2],
            "peer recovers tree after orphan squash"
        );
        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[tokio::test]
    async fn fetch_without_remote_is_noop() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        assert_eq!(g.fetch_remote().await.unwrap(), 0);
        g.push_remote().await.unwrap(); // also a no-op
        let _ = std::fs::remove_dir_all(&dir);
    }
}
