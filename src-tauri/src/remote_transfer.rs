use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use russh::{ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};
use tokio::time::{interval, MissedTickBehavior};

use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::{
    connect_authenticated_with_agent_forwarding_sock, SshError, SshProfile, SshTransportSettings,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectTransferTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DirectTransferKind {
    File,
    Dir,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectTransferOutput {
    pub method: &'static str,
    pub exit_status: u32,
    pub stdout: String,
    pub stderr: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy)]
pub struct DirectProgressHint {
    pub percent: Option<u8>,
    pub transferred_hint: Option<u64>,
}

pub type DirectProgressFn = Arc<dyn Fn(DirectProgressHint) + Send + Sync>;
pub type DirectHeartbeatFn = Arc<dyn Fn() + Send + Sync>;

/// Run source-exec Direct transfer with agent forwarding (no key file on source).
///
/// `agent_sock`: when set, forwarded agent channels use this socket (ephemeral
/// agent holding the destination key). When `None`, the system agent is used.
#[allow(clippy::too_many_arguments)]
pub async fn run_direct_transfer(
    source_profile: &SshProfile,
    source_path: &str,
    kind: DirectTransferKind,
    target: &DirectTransferTarget,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
    agent_sock: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
    on_heartbeat: Option<DirectHeartbeatFn>,
    on_progress: Option<DirectProgressFn>,
) -> Result<DirectTransferOutput, SshError> {
    let handle = connect_authenticated_with_agent_forwarding_sock(
        source_profile,
        known_hosts,
        transport,
        agent_sock,
    )
    .await?;

    let command = build_direct_transfer_command(source_path, kind, target);

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(format!("direct transfer channel: {e}")))?;

    channel
        .agent_forward(true)
        .await
        .map_err(|e| SshError::Channel(format!("agent forward request: {e}")))?;

    channel
        .exec(true, command.as_str())
        .await
        .map_err(|e| SshError::Channel(format!("direct transfer exec: {e}")))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut tick = interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Skip the immediate first tick so we don't spam before any output.
    tick.tick().await;

    loop {
        if cancel.as_ref().is_some_and(|f| f.load(Ordering::Relaxed)) {
            let _ = channel.close().await;
            let _ = handle
                .disconnect(Disconnect::ByApplication, "direct transfer canceled", "en")
                .await;
            return Err(SshError::Channel("direct transfer canceled".into()));
        }

        tokio::select! {
            biased;
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        append_capped(&mut stdout, data);
                        if let Some(ref cb) = on_progress {
                            if let Some(hint) = parse_rsync_progress(data) {
                                cb(hint);
                            }
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, ext: _ }) => {
                        append_capped(&mut stderr, data);
                        if let Some(ref cb) = on_progress {
                            if let Some(hint) = parse_rsync_progress(data) {
                                cb(hint);
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: code }) => {
                        exit_status = Some(code);
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    Some(_) => {}
                }
            }
            _ = tick.tick() => {
                if let Some(ref hb) = on_heartbeat {
                    hb();
                }
            }
        }
    }

    let _ = handle
        .disconnect(Disconnect::ByApplication, "direct transfer complete", "en")
        .await;

    finish_direct_transfer_result(command, stdout, stderr, exit_status)
}

fn finish_direct_transfer_result(
    command: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_status: Option<u32>,
) -> Result<DirectTransferOutput, SshError> {
    let stdout = String::from_utf8_lossy(&stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
    let Some(exit_status) = exit_status else {
        let detail = if !stderr.is_empty() { &stderr } else { &stdout };
        return Err(SshError::Channel(if detail.is_empty() {
            "direct transfer did not report an exit status".into()
        } else {
            format!("direct transfer did not report an exit status: {detail}")
        }));
    };
    if exit_status != 0 {
        let detail = if !stderr.is_empty() { &stderr } else { &stdout };
        return Err(SshError::Channel(if detail.is_empty() {
            format!("direct transfer exited with status {exit_status}")
        } else {
            format!("direct transfer exited with status {exit_status}: {detail}")
        }));
    }

    Ok(DirectTransferOutput {
        method: "direct",
        exit_status,
        stdout,
        stderr,
        command,
    })
}

/// Build the source-side shell that prefers `rsync`, falls back to `scp`.
/// Auth to the destination uses the forwarded agent (no `-i` key path).
pub fn build_direct_transfer_command(
    source_path: &str,
    kind: DirectTransferKind,
    target: &DirectTransferTarget,
) -> String {
    let ssh_command =
        "ssh -p {port} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10"
            .replace("{port}", &target.port.to_string());
    let scp_command =
        "scp -P {port} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 -p -r"
            .replace("{port}", &target.port.to_string());
    let remote_login = format!("{}@{}", target.user, target.host);
    let rsync_dest_path = match kind {
        DirectTransferKind::File => target.path.clone(),
        DirectTransferKind::Dir => ensure_trailing_slash(&target.path),
    };
    let rsync_source_path = match kind {
        DirectTransferKind::File => source_path.to_string(),
        DirectTransferKind::Dir => ensure_trailing_slash(source_path),
    };
    let mkdir_path = match kind {
        DirectTransferKind::File => parent_remote_path(&target.path),
        DirectTransferKind::Dir => target.path.clone(),
    };
    let remote_dest = format!("{remote_login}:{rsync_dest_path}");
    let rsync_path = format!("mkdir -p -- {} && rsync", shell_quote(&mkdir_path));
    let ssh_mkdir = format!("mkdir -p -- {}", shell_quote(&mkdir_path));

    format!(
        "if command -v rsync >/dev/null 2>&1 && command -v ssh >/dev/null 2>&1; then \
         rsync -a --partial --info=progress2 --rsync-path={} -e {} -- {} {}; \
         elif command -v scp >/dev/null 2>&1 && command -v ssh >/dev/null 2>&1; then \
         {} {} {} && {} -- {} {}; \
         else echo 'AeroTab direct transfer requires rsync or scp on the source host' >&2; exit 127; fi",
        double_quote(&rsync_path),
        shell_quote(&ssh_command),
        shell_quote(&rsync_source_path),
        shell_quote(&remote_dest),
        ssh_command,
        shell_quote(&remote_login),
        double_quote(&ssh_mkdir),
        scp_command,
        shell_quote(source_path),
        shell_quote(&format!("{remote_login}:{}", target.path)),
    )
}

/// Light source-side tool check used by `sftp.directPreflight`.
pub async fn probe_source_transfer_tools(
    source_profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
) -> Result<(bool, bool), SshError> {
    let handle = connect_authenticated_with_agent_forwarding_sock(
        source_profile,
        known_hosts,
        transport,
        None,
    )
    .await?;
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(format!("preflight channel: {e}")))?;
    let cmd = "command -v rsync >/dev/null 2>&1; echo RSYNC:$?; command -v scp >/dev/null 2>&1; echo SCP:$?; command -v ssh >/dev/null 2>&1; echo SSH:$?";
    channel
        .exec(true, cmd)
        .await
        .map_err(|e| SshError::Channel(format!("preflight exec: {e}")))?;
    let mut stdout = Vec::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => append_capped(&mut stdout, data),
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    let _ = handle
        .disconnect(Disconnect::ByApplication, "preflight complete", "en")
        .await;
    let text = String::from_utf8_lossy(&stdout);
    let has_rsync = text.contains("RSYNC:0");
    let has_scp = text.contains("SCP:0");
    let has_ssh = text.contains("SSH:0");
    Ok((has_rsync && has_ssh, has_scp && has_ssh))
}

pub fn source_host_consent_key(profile: &SshProfile) -> String {
    format!("{}@{}:{}", profile.user, profile.host, profile.port)
}

pub fn shell_quote(input: &str) -> String {
    if input.is_empty() {
        return "''".into();
    }
    format!("'{}'", input.replace('\'', "'\\''"))
}

fn double_quote(input: &str) -> String {
    let mut out = String::from("\"");
    for ch in input.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn parent_remote_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".into();
    }
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(idx) => trimmed[..idx].into(),
        None => ".".into(),
    }
}

fn ensure_trailing_slash(path: &str) -> String {
    if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

fn append_capped(out: &mut Vec<u8>, data: &[u8]) {
    const MAX_CAPTURED_OUTPUT: usize = 64 * 1024;
    let remaining = MAX_CAPTURED_OUTPUT.saturating_sub(out.len());
    if remaining > 0 {
        out.extend_from_slice(&data[..data.len().min(remaining)]);
    }
}

/// Parse `rsync --info=progress2` lines for a percent / byte hint.
fn parse_rsync_progress(data: &[u8]) -> Option<DirectProgressHint> {
    let text = String::from_utf8_lossy(data);
    let mut percent = None;
    let mut transferred_hint = None;
    for token in text.split_whitespace() {
        if let Some(p) = token.strip_suffix('%') {
            if let Ok(v) = p.parse::<u8>() {
                if v <= 100 {
                    percent = Some(v);
                }
            }
        } else if token.chars().all(|c| c.is_ascii_digit() || c == ',') {
            let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = digits.parse::<u64>() {
                if v > 0 {
                    transferred_hint = Some(v);
                }
            }
        }
    }
    if percent.is_some() || transferred_hint.is_some() {
        Some(DirectProgressHint {
            percent,
            transferred_hint,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_rsync_first_file_transfer_command() {
        let target = DirectTransferTarget {
            user: "deploy".into(),
            host: "10.0.0.8".into(),
            port: 2222,
            path: "/var/www/app config.tar".into(),
        };

        let command =
            build_direct_transfer_command("/tmp/app config.tar", DirectTransferKind::File, &target);

        assert!(command.contains("command -v rsync"));
        assert!(command.contains("BatchMode=yes"));
        assert!(command.contains("-p 2222"));
        assert!(command.contains("mkdir -p -- '/var/www'"));
        assert!(command.contains("'deploy@10.0.0.8:/var/www/app config.tar'"));
        assert!(command.contains("'/tmp/app config.tar'"));
        assert!(!command.contains("-i "));
        assert!(!command.contains("IdentitiesOnly"));
    }

    #[test]
    fn directory_transfer_copies_contents_into_created_target_dir() {
        let target = DirectTransferTarget {
            user: "root".into(),
            host: "example.internal".into(),
            port: 22,
            path: "/srv/backups/data set".into(),
        };

        let command =
            build_direct_transfer_command("/opt/data set", DirectTransferKind::Dir, &target);

        assert!(command.contains("mkdir -p -- '/srv/backups/data set'"));
        assert!(command.contains("'/opt/data set/'"));
        assert!(command.contains("'root@example.internal:/srv/backups/data set/'"));
    }

    #[test]
    fn missing_exit_status_is_not_success() {
        let err = finish_direct_transfer_result("echo test".into(), Vec::new(), Vec::new(), None)
            .expect_err("missing exit status must fail");
        assert!(err.to_string().contains("did not report an exit status"));
    }

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn parses_rsync_progress2_line() {
        let hint = parse_rsync_progress(b"  12,345,678  45%  1.23MB/s    0:00:12").unwrap();
        assert_eq!(hint.percent, Some(45));
        assert_eq!(hint.transferred_hint, Some(12_345_678));
    }
}
