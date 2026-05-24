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
use std::process::Command;
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
                author_email: "aerotab@localhost".into(),
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
        let url = normalize_git_remote_url(&url.into());
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
        tokio::time::timeout(std::time::Duration::from_secs(120), async move {
            tokio::task::spawn_blocking(move || me.fetch_blocking())
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
            tokio::task::spawn_blocking(move || me.push_blocking())
                .await
                .map_err(|e| SyncError::Transport(format!("join: {e}")))?
        })
        .await
        .map_err(|_| SyncError::Transport("git push timed out after 120s".into()))?
    }

    fn fetch_blocking(&self) -> Result<usize, SyncError> {
        let Some(remote_cfg) = self.inner.remote.clone() else {
            return Ok(0);
        };
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
        let head_target = repo.head().ok().and_then(|h| h.target());
        if analysis.0.is_fast_forward() {
            let n = checkout_branch_to_commit(&repo, &remote_cfg, fetch_commit.id(), "fast-forward")?;
            if let Some(old) = head_target {
                let count = count_commits_between(&repo, old, fetch_commit.id()).unwrap_or(0);
                return Ok(count.max(n));
            }
            return Ok(n.max(1));
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
            return Ok(count.max(n));
        }
        Ok(n.max(1))
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
                &["fetch", &remote_cfg.name, refspec],
            );
        }
        match self.fetch_remote_refs_libgit2(remote_cfg, refspec) {
            Ok(()) => Ok(()),
            Err(e) if is_ssh_remote_url(&remote_cfg.url) && transport_is_ssh_banner_failure(&e) => {
                run_git(
                    &self.inner.repo_path,
                    &remote_cfg.auth,
                    ssh_port,
                    &["fetch", &remote_cfg.name, refspec],
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
        let mut remote = repo.find_remote(&remote_cfg.name).map_err(git_err)?;
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
                &["push", &remote_cfg.name, refspec],
            );
        }
        match self.push_remote_refs_libgit2(remote_cfg, refspec) {
            Ok(()) => Ok(()),
            Err(e) if is_ssh_remote_url(&remote_cfg.url) && transport_is_ssh_banner_failure(&e) => {
                run_git(
                    &self.inner.repo_path,
                    &remote_cfg.auth,
                    ssh_port,
                    &["push", &remote_cfg.name, refspec],
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
        let mut remote = repo.find_remote(&remote_cfg.name).map_err(git_err)?;
        let mut cb = RemoteCallbacks::new();
        let auth = remote_cfg.auth.clone();
        cb.credentials(move |url, user, allowed| build_creds(&auth, url, user, allowed));
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
    repo.checkout_head(Some(
        git2::build::CheckoutBuilder::default().force(),
    ))
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
        let url = format!("file://{}", bare.display());

        let a_dir = tmpdir();
        let id_a = RecordId(Uuid::new_v4());
        let a = GitBackend::open_or_init(&a_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        a.put(Group::Appearance, id_a, b"from-a").await.unwrap();
        a.push_remote().await.unwrap();

        let b_dir = tmpdir();
        let id_b = RecordId(Uuid::new_v4());
        let b = GitBackend::open_or_init(&b_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        b.fetch_remote().await.unwrap();
        b.put(Group::Appearance, id_b, b"from-b").await.unwrap();

        let a2 = GitBackend::open_or_init(&a_dir)
            .unwrap()
            .with_remote("origin", &url, "master")
            .unwrap();
        let id_a2 = RecordId(Uuid::new_v4());
        a2.put(Group::Appearance, id_a2, b"from-a2").await.unwrap();
        a2.push_remote().await.unwrap();

        // Diverged: B has a local-only commit; remote moved on A.
        b.fetch_remote().await.expect("fetch should realign, not error");
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
    async fn fetch_without_remote_is_noop() {
        let dir = tmpdir();
        let g = GitBackend::open_or_init(&dir).unwrap();
        assert_eq!(g.fetch_remote().await.unwrap(), 0);
        g.push_remote().await.unwrap(); // also a no-op
        let _ = std::fs::remove_dir_all(&dir);
    }
}
