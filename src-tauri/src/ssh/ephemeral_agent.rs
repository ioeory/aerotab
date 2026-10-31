//! In-process SSH agent used for Direct transfer agent-forwarding.
//!
//! Destination private keys are held only in AeroTab's memory and served to the
//! **source** host over forwarded agent channels, so `rsync`/`scp` never see a
//! key file and no external `ssh-agent` process is required on any platform.

use std::sync::Arc;

use russh::keys::key::KeyPair;
use russh_keys::agent::client::AgentClient;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ssh::{AuthMethod, SshError};

const AGENT_PIPE_CAPACITY: usize = 64 * 1024;

struct ServeTask(JoinHandle<()>);

impl Drop for ServeTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Cheaply cloneable handle; the agent stops when the last clone is dropped.
#[derive(Clone)]
pub struct InMemoryAgent {
    tx: mpsc::UnboundedSender<DuplexStream>,
    _task: Arc<ServeTask>,
}

impl InMemoryAgent {
    /// Start an empty agent and load `key` into it.
    pub async fn with_key(key: &KeyPair) -> Result<Self, SshError> {
        let (tx, rx) = mpsc::unbounded_channel::<DuplexStream>();
        let listener = Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|stream| (Ok(stream), rx))
        }));
        let task = tokio::spawn(async move {
            if let Err(e) = russh_keys::agent::server::serve(listener, ()).await {
                tracing::warn!("in-memory ssh agent stopped: {e}");
            }
        });
        let agent = Self {
            tx,
            _task: Arc::new(ServeTask(task)),
        };
        let mut client = AgentClient::connect(agent.connect()?);
        client
            .add_identity(key, &[])
            .await
            .map_err(|e| SshError::Agent(format!("load key into in-memory agent: {e}")))?;
        Ok(agent)
    }

    /// Open a new connection to the agent (one per forwarded agent channel).
    pub fn connect(&self) -> Result<DuplexStream, SshError> {
        let (ours, theirs) = tokio::io::duplex(AGENT_PIPE_CAPACITY);
        self.tx
            .send(theirs)
            .map_err(|_| SshError::Agent("in-memory agent is not running".into()))?;
        Ok(ours)
    }
}

/// Load the destination private key for Direct from `auth`.
///
/// `Ok(None)` means the system agent should be used (`AuthMethod::Agent`).
pub fn dest_key_for_direct(auth: &AuthMethod) -> Result<Option<KeyPair>, SshError> {
    match auth {
        AuthMethod::Agent => Ok(None),
        AuthMethod::PublicKey {
            key_path,
            passphrase,
        } => russh::keys::load_secret_key(key_path, passphrase.as_deref())
            .map(Some)
            .map_err(|e| SshError::Agent(format!("load {}: {e}", key_path.display()))),
        AuthMethod::PrivateKeyMemory { pem, passphrase } => {
            russh::keys::decode_secret_key(pem, passphrase.as_deref())
                .map(Some)
                .map_err(|e| SshError::Agent(format!("decode vault key: {e}")))
        }
        AuthMethod::VaultRef { .. } => Err(SshError::Connect(
            "destination vault auth was not resolved; reopen SFTP session".into(),
        )),
        AuthMethod::Password { .. } => Err(SshError::Agent(
            "destination password auth requires Relay".into(),
        )),
    }
}

/// Whether Direct can use agent-forward auth for this destination profile.
pub fn dest_auth_supports_direct(auth: &AuthMethod) -> bool {
    matches!(
        auth,
        AuthMethod::Agent
            | AuthMethod::PublicKey { .. }
            | AuthMethod::PrivateKeyMemory { .. }
            | AuthMethod::VaultRef { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_agent_lists_loaded_key() {
        let key = KeyPair::generate_ed25519().expect("ed25519 key");
        let agent = InMemoryAgent::with_key(&key).await.expect("agent");
        let mut client = AgentClient::connect(agent.connect().expect("connect"));
        let ids = client.request_identities().await.expect("identities");
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn password_dest_is_not_direct_capable() {
        assert!(!dest_auth_supports_direct(&AuthMethod::Password {
            secret: "x".into()
        }));
        assert!(dest_auth_supports_direct(&AuthMethod::Agent));
    }
}
