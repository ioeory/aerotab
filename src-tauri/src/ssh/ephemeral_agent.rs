//! Short-lived local ssh-agent used for Direct transfer agent-forwarding.
//!
//! Destination private keys are loaded here and forwarded to the **source** host
//! so `rsync`/`scp` never receive a key file under `/tmp` on the source.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::ssh::SshError;

/// RAII wrapper around a spawned `ssh-agent` that holds one or more identities.
pub struct EphemeralAgent {
    sock: String,
    pid: u32,
}

impl EphemeralAgent {
    pub fn sock(&self) -> &str {
        &self.sock
    }

    /// Start a private agent and `ssh-add` `key_path` (optional passphrase).
    pub async fn spawn_with_key(
        key_path: &Path,
        passphrase: Option<&str>,
    ) -> Result<Self, SshError> {
        if !key_path.is_file() {
            return Err(SshError::Io(format!(
                "destination key not found: {}",
                key_path.display()
            )));
        }
        let agent = Self::spawn_agent().await?;
        agent.add_key(key_path, passphrase).await?;
        Ok(agent)
    }

    async fn spawn_agent() -> Result<Self, SshError> {
        let output = Command::new("ssh-agent")
            .arg("-s")
            .output()
            .await
            .map_err(|e| SshError::Agent(format!("spawn ssh-agent: {e}")))?;
        if !output.status.success() {
            return Err(SshError::Agent(format!(
                "ssh-agent failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let sock = parse_agent_env(&stdout, "SSH_AUTH_SOCK")
            .ok_or_else(|| SshError::Agent("ssh-agent did not print SSH_AUTH_SOCK".into()))?;
        let pid_str = parse_agent_env(&stdout, "SSH_AGENT_PID")
            .ok_or_else(|| SshError::Agent("ssh-agent did not print SSH_AGENT_PID".into()))?;
        let pid: u32 = pid_str
            .parse()
            .map_err(|_| SshError::Agent(format!("invalid SSH_AGENT_PID: {pid_str}")))?;
        Ok(Self { sock, pid })
    }

    async fn add_key(&self, key_path: &Path, passphrase: Option<&str>) -> Result<(), SshError> {
        let key_path = key_path.to_path_buf();
        if let Some(pass) = passphrase.filter(|p| !p.is_empty()) {
            self.add_key_with_passphrase(&key_path, pass).await
        } else {
            let status = Command::new("ssh-add")
                .env("SSH_AUTH_SOCK", &self.sock)
                .arg(&key_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .status()
                .await
                .map_err(|e| SshError::Agent(format!("ssh-add: {e}")))?;
            if !status.success() {
                return Err(SshError::Agent(
                    "ssh-add failed (is the key passphrase-protected?)".into(),
                ));
            }
            Ok(())
        }
    }

    async fn add_key_with_passphrase(
        &self,
        key_path: &Path,
        passphrase: &str,
    ) -> Result<(), SshError> {
        let askpass = write_askpass_script(passphrase).await?;
        let status = Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &self.sock)
            .env("SSH_ASKPASS", &askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "1")
            .arg(key_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .await;
        let _ = tokio::fs::remove_file(&askpass).await;
        let status = status.map_err(|e| SshError::Agent(format!("ssh-add: {e}")))?;
        if !status.success() {
            return Err(SshError::Agent(
                "ssh-add failed with passphrase (check key/passphrase)".into(),
            ));
        }
        Ok(())
    }
}

impl Drop for EphemeralAgent {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("kill")
                .arg(self.pid.to_string())
                .status();
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.pid.to_string(), "/F"])
                .status();
        }
    }
}

fn parse_agent_env(stdout: &str, key: &str) -> Option<String> {
    // SSH_AUTH_SOCK=/tmp/ssh-xxx/agent.123; export SSH_AUTH_SOCK;
    for part in stdout.split([';', '\n']) {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(&format!("{key}=")) {
            let value = rest.trim().trim_matches(';').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

async fn write_askpass_script(passphrase: &str) -> Result<PathBuf, SshError> {
    let dir = std::env::temp_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(format!("aerotab-askpass-{}.sh", uuid::Uuid::new_v4()));
        let escaped = passphrase.replace('\'', "'\\''");
        let body = format!("#!/bin/sh\nprintf '%s' '{escaped}'\n");
        tokio::fs::write(&path, body)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        let mut perms = tokio::fs::metadata(&path)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?
            .permissions();
        perms.set_mode(0o700);
        tokio::fs::set_permissions(&path, perms)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        Ok(path)
    }
    #[cfg(windows)]
    {
        let path = dir.join(format!("aerotab-askpass-{}.cmd", uuid::Uuid::new_v4()));
        let escaped = passphrase.replace('"', "\"\"");
        let body = format!("@echo off\necho {escaped}\n");
        tokio::fs::write(&path, body)
            .await
            .map_err(|e| SshError::Io(e.to_string()))?;
        Ok(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = passphrase;
        Err(SshError::Agent(
            "askpass unsupported on this platform".into(),
        ))
    }
}

/// Whether Direct can use agent-forward auth for this destination profile.
pub fn dest_auth_supports_direct(auth: &crate::ssh::AuthMethod) -> bool {
    matches!(
        auth,
        crate::ssh::AuthMethod::Agent
            | crate::ssh::AuthMethod::PublicKey { .. }
            | crate::ssh::AuthMethod::VaultRef { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_agent_s_output() {
        let sample = "SSH_AUTH_SOCK=/tmp/ssh-abc/agent.1; export SSH_AUTH_SOCK;\n\
                      SSH_AGENT_PID=12345; export SSH_AGENT_PID;\n\
                      echo Agent pid 12345;";
        assert_eq!(
            parse_agent_env(sample, "SSH_AUTH_SOCK").as_deref(),
            Some("/tmp/ssh-abc/agent.1")
        );
        assert_eq!(
            parse_agent_env(sample, "SSH_AGENT_PID").as_deref(),
            Some("12345")
        );
    }
}
