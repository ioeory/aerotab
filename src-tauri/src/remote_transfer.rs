use std::path::Path;
use std::time::Duration;

use russh::{ChannelMsg, Disconnect};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use uuid::Uuid;

use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::{connect_authenticated, SshError, SshProfile, SshTransportSettings};

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

#[allow(clippy::too_many_arguments)]
pub async fn run_direct_transfer(
    source_profile: &SshProfile,
    source_path: &str,
    kind: DirectTransferKind,
    target: &DirectTransferTarget,
    known_hosts: Option<KnownHosts>,
    transport: SshTransportSettings,
    timeout_duration: Duration,
    dest_key_path: Option<&Path>,
) -> Result<DirectTransferOutput, SshError> {
    let handle = connect_authenticated(source_profile, known_hosts, transport).await?;

    // If we have a dest key, upload it to the source host so rsync/scp can use -i.
    let remote_key_path: Option<String> = if let Some(key_path) = dest_key_path {
        let key_content = std::fs::read(key_path).map_err(|e| SshError::Io(e.to_string()))?;
        let remote = format!("/tmp/aerotab-transfer-key-{}", Uuid::new_v4());
        let upload_cmd = format!(
            "cat > {} && chmod 600 {}",
            shell_quote(&remote),
            shell_quote(&remote),
        );
        let mut ch = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(format!("key upload channel: {e}")))?;
        ch.exec(true, upload_cmd.as_str())
            .await
            .map_err(|e| SshError::Channel(format!("key upload exec: {e}")))?;
        ch.data(key_content.as_slice())
            .await
            .map_err(|e| SshError::Channel(format!("key upload write: {e}")))?;
        ch.eof()
            .await
            .map_err(|e| SshError::Channel(format!("key upload eof: {e}")))?;
        let mut exit = None;
        while let Some(msg) = ch.wait().await {
            if let ChannelMsg::ExitStatus { exit_status } = msg {
                exit = Some(exit_status);
            }
            if matches!(msg, ChannelMsg::Eof | ChannelMsg::Close) {
                break;
            }
        }
        if exit != Some(0) {
            return Err(SshError::Channel(
                "failed to upload destination key to source host".into(),
            ));
        }
        Some(remote)
    } else {
        None
    };

    let command =
        build_direct_transfer_command(source_path, kind, target, remote_key_path.as_deref());

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(format!("direct transfer channel: {e}")))?;
    channel
        .exec(true, command.as_str())
        .await
        .map_err(|e| SshError::Channel(format!("direct transfer exec: {e}")))?;

    let collect = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => append_capped(&mut stdout, data),
                ChannelMsg::ExtendedData { ref data, ext: _ } => append_capped(&mut stderr, data),
                ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok::<_, SshError>((stdout, stderr, exit_status))
    };

    let (stdout, stderr, exit_status) = timeout(timeout_duration, collect)
        .await
        .map_err(|_| SshError::Channel("direct transfer timed out".into()))??;

    // Clean up the temp key file from the source host.
    if let Some(ref remote_key) = remote_key_path {
        let cleanup_cmd = format!("rm -f {}", shell_quote(remote_key));
        if let Ok(mut ch) = handle.channel_open_session().await {
            if ch.exec(true, cleanup_cmd.as_str()).await.is_ok() {
                while let Some(msg) = ch.wait().await {
                    if matches!(msg, ChannelMsg::Eof | ChannelMsg::Close) {
                        break;
                    }
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

pub fn build_direct_transfer_command(
    source_path: &str,
    kind: DirectTransferKind,
    target: &DirectTransferTarget,
    dest_key_path: Option<&str>,
) -> String {
    let identity_opt = dest_key_path
        .map(|p| format!("-i {} -o IdentitiesOnly=yes", shell_quote(p)))
        .unwrap_or_default();
    let ssh_command = format!(
        "ssh -p {} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 {}",
        target.port, identity_opt,
    );
    let scp_command = format!(
        "scp -P {} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 -p -r {}",
        target.port, identity_opt,
    );
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

        let command = build_direct_transfer_command(
            "/tmp/app config.tar",
            DirectTransferKind::File,
            &target,
            None,
        );

        assert!(command.contains("command -v rsync"));
        assert!(command.contains("BatchMode=yes"));
        assert!(command.contains("-p 2222"));
        assert!(command.contains("mkdir -p -- '/var/www'"));
        assert!(command.contains("'deploy@10.0.0.8:/var/www/app config.tar'"));
        assert!(command.contains("'/tmp/app config.tar'"));
    }

    #[test]
    fn builds_command_with_key_path() {
        let target = DirectTransferTarget {
            user: "deploy".into(),
            host: "10.0.0.8".into(),
            port: 2222,
            path: "/var/www/data".into(),
        };

        let command = build_direct_transfer_command(
            "/tmp/data",
            DirectTransferKind::File,
            &target,
            Some("/tmp/my-key.pem"),
        );

        assert!(command.contains("-i '/tmp/my-key.pem'"));
        assert!(command.contains("-o IdentitiesOnly=yes"));
        assert!(command.contains("BatchMode=yes"));
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
            build_direct_transfer_command("/opt/data set", DirectTransferKind::Dir, &target, None);

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
}
