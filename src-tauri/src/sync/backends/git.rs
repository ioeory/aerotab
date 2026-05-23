//! Git sync backend.
//!
//! On-disk layout inside the working tree:
//!
//! ```text
//! <repo>/<group>/<record-uuid>.bin
//! ```
//!
//! Every [`put`](SyncBackend::put) / [`delete`](SyncBackend::delete) creates a
//! commit. [`sync_remote`](GitBackend::sync_remote) pulls + pushes against the
//! configured `origin`. Remote auth is provided through a closure so callers
//! can plug in SSH-agent / HTTPS tokens without this module taking a hard
//! dependency on either.
//!
//! All git2 calls are blocking; we wrap them in `tokio::task::spawn_blocking`
//! so the async trait stays honest.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use git2::{
    Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository, Signature,
};
use uuid::Uuid;

use crate::sync::{Group, RecordId, SyncBackend, SyncError};

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
                author_email: "tabby@localhost".into(),
                remote: None,
            }),
        })
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
        let url = url.into();
        let branch = branch.into();
        // Register the remote with git itself so cli tools see it too.
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        match repo.find_remote(&name) {
            Ok(existing) => {
                if existing.url() != Some(url.as_str()) {
                    repo.remote_set_url(&name, &url).map_err(git_err)?;
                }
            }
            Err(_) => {
                repo.remote(&name, &url).map_err(git_err)?;
            }
        }
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
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.fetch_blocking())
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    /// Pushes the current branch to the configured remote. No-op when no
    /// remote is set.
    pub async fn push_remote(&self) -> Result<(), SyncError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.push_blocking())
            .await
            .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    fn fetch_blocking(&self) -> Result<usize, SyncError> {
        let Some(remote_cfg) = self.inner.remote.clone() else {
            return Ok(0);
        };
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        let mut remote = repo.find_remote(&remote_cfg.name).map_err(git_err)?;
        let mut cb = RemoteCallbacks::new();
        let auth = remote_cfg.auth.clone();
        cb.credentials(move |_url, user, allowed| build_creds(&auth, user, allowed));
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(cb);
        let refspec = format!(
            "+refs/heads/{branch}:refs/remotes/{name}/{branch}",
            branch = remote_cfg.branch,
            name = remote_cfg.name
        );
        remote
            .fetch(&[refspec.as_str()], Some(&mut opts), None)
            .map_err(git_err)?;

        // Fast-forward HEAD if possible.
        let remote_ref = format!("refs/remotes/{}/{}", remote_cfg.name, remote_cfg.branch);
        let fetch_head = match repo.find_reference(&remote_ref) {
            Ok(r) => r,
            Err(_) => return Ok(0),
        };
        let fetch_commit = fetch_head.peel_to_commit().map_err(git_err)?;
        let analysis = {
            let annotated = repo
                .reference_to_annotated_commit(&fetch_head)
                .map_err(git_err)?;
            repo.merge_analysis(&[&annotated]).map_err(git_err)?
        };
        if analysis.0.is_up_to_date() {
            return Ok(0);
        }
        if analysis.0.is_fast_forward() {
            let head_target = repo.head().ok().and_then(|h| h.target());
            let local_branch_ref = format!("refs/heads/{}", remote_cfg.branch);
            let mut local_ref = match repo.find_reference(&local_branch_ref) {
                Ok(r) => r,
                Err(_) => repo
                    .reference(
                        &local_branch_ref,
                        fetch_commit.id(),
                        true,
                        "init local branch",
                    )
                    .map_err(git_err)?,
            };
            local_ref
                .set_target(fetch_commit.id(), "fast-forward")
                .map_err(git_err)?;
            repo.set_head(&local_branch_ref).map_err(git_err)?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .map_err(git_err)?;
            // Rough commit count between old and new HEAD.
            if let Some(old) = head_target {
                let count = count_commits_between(&repo, old, fetch_commit.id()).unwrap_or(0);
                return Ok(count);
            }
            return Ok(1);
        }
        // True merges (history divergence) are not handled by the git
        // backend; the engine's record-level merge resolves data conflicts
        // and a subsequent fast-forward push will succeed once the local
        // branch is rewritten. For now surface a clear error so callers
        // know the operator needs to intervene.
        Err(SyncError::Transport(
            "diverged history; manual reconcile required".into(),
        ))
    }

    fn push_blocking(&self) -> Result<(), SyncError> {
        let Some(remote_cfg) = self.inner.remote.clone() else {
            return Ok(());
        };
        let repo = Repository::open(&self.inner.repo_path).map_err(git_err)?;
        // Ensure local branch points to HEAD (we always commit on HEAD).
        let head = repo.head().map_err(git_err)?;
        let head_oid = head
            .target()
            .ok_or_else(|| SyncError::Transport("HEAD has no target (empty repo?)".into()))?;
        let local_branch_ref = format!("refs/heads/{}", remote_cfg.branch);
        // Force-create or update the named branch ref.
        let _ = repo.reference(&local_branch_ref, head_oid, true, "align push branch");

        let mut remote = repo.find_remote(&remote_cfg.name).map_err(git_err)?;
        let mut cb = RemoteCallbacks::new();
        let auth = remote_cfg.auth.clone();
        cb.credentials(move |_url, user, allowed| build_creds(&auth, user, allowed));
        let mut opts = PushOptions::new();
        opts.remote_callbacks(cb);
        let refspec = format!(
            "refs/heads/{branch}:refs/heads/{branch}",
            branch = remote_cfg.branch
        );
        remote
            .push(&[refspec.as_str()], Some(&mut opts))
            .map_err(git_err)?;
        Ok(())
    }
}

fn build_creds(
    auth: &RemoteAuth,
    user_from_url: Option<&str>,
    allowed: git2::CredentialType,
) -> Result<Cred, git2::Error> {
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        if let Some(path) = &auth.ssh_key_path {
            let user = user_from_url.or(auth.username.as_deref()).unwrap_or("git");
            return Cred::ssh_key(user, None, path, auth.ssh_passphrase.as_deref());
        }
    }
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        let user = auth.username.as_deref().unwrap_or("git");
        let pw = auth.password.as_deref().unwrap_or("");
        return Cred::userpass_plaintext(user, pw);
    }
    if allowed.contains(git2::CredentialType::DEFAULT) {
        return Cred::default();
    }
    Err(git2::Error::from_str("no usable credentials"))
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
    SyncError::Transport(format!("git: {e}"))
}

fn group_segment(group: Group) -> &'static str {
    match group {
        Group::Connections => "connections",
        Group::Appearance => "appearance",
        Group::Shortcuts => "shortcuts",
        Group::PluginCfg => "plugincfg",
        Group::Credentials => "credentials",
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
        let msg = format!("put {}/{}", group_segment(group), id.0);
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            std::fs::create_dir_all(&dir).map_err(io_err)?;
            // Atomic write via tmp + rename.
            let tmp = path.with_extension("bin.tmp");
            std::fs::write(&tmp, &blob).map_err(io_err)?;
            std::fs::rename(&tmp, &path).map_err(io_err)?;
            me.commit_all(&msg)?;
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }

    async fn delete(&self, group: Group, id: RecordId) -> Result<(), SyncError> {
        let path = self.record_path(group, id);
        let me = self.clone();
        let msg = format!("delete {}/{}", group_segment(group), id.0);
        tokio::task::spawn_blocking(move || -> Result<(), SyncError> {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(io_err(e)),
            }
            me.commit_all(&msg)?;
            Ok(())
        })
        .await
        .map_err(|e| SyncError::Transport(format!("join: {e}")))?
    }
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

    fn tmpdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tabby-git-test-{}", Uuid::new_v4()));
        p
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
    async fn put_creates_commits() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        let id1 = RecordId(Uuid::new_v4());
        let id2 = RecordId(Uuid::new_v4());
        g.put(Group::Appearance, id1, b"a").await.unwrap();
        g.put(Group::Appearance, id2, b"b").await.unwrap();
        let repo = Repository::open(&dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        let count = walk.count();
        assert_eq!(count, 2, "two commits expected, got {count}");
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
        let url = format!("file://{}", bare.display());

        // Workspace A: produce two commits, push.
        let a_dir = tmpdir();
        let a = GitBackend::open_or_init(&a_dir).unwrap();
        // First commit so HEAD exists before we attach a remote.
        a.put(Group::Appearance, RecordId(Uuid::new_v4()), b"first")
            .await
            .unwrap();
        let a = a.with_remote("origin", &url, "master").unwrap();
        a.push_remote().await.unwrap();

        // Workspace B: clone state into a fresh tree, then fetch.
        let b_dir = tmpdir();
        let b = GitBackend::open_or_init(&b_dir).unwrap();
        let b = b.with_remote("origin", &url, "master").unwrap();
        let n = b.fetch_remote().await.unwrap();
        assert!(n >= 1, "expected at least one fast-forwarded commit");
        // The file pushed by A is now visible to B.
        let listed = b.list(Group::Appearance).await.unwrap();
        assert!(!listed.is_empty(), "B should see A's record after fetch");

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
