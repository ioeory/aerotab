//! Device-local network definitions. This database is intentionally outside settings/sync.
//! RPC integration: proxy.list/save/delete and forwarding.list/save/delete/start/stop/running.
//! Save accepts { resource } / { rule }; delete/start/stop accept { id }.
//! Profile/Vault deletion must consult `profile_dependents` / `vault_dependents`.
use crate::ssh::{
    known_hosts::KnownHosts,
    tunnel::{TunnelKind, TunnelManager, TunnelMeta, TunnelOpenRequest},
    SshProfile,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, NetworkError>;
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("network storage: {0}")]
    Storage(String),
    #[error("invalid network resource: {0}")]
    Invalid(String),
    #[error("resource is referenced by: {0}")]
    Referenced(String),
}
impl From<sled::Error> for NetworkError {
    fn from(e: sled::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
impl From<serde_json::Error> for NetworkError {
    fn from(e: serde_json::Error) -> Self {
        Self::Storage(e.to_string())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyConfig {
    SshJump {
        profile_id: Uuid,
    },
    Socks5 {
        host: String,
        port: u16,
        #[serde(default)]
        vault_entry_id: Option<String>,
    },
    HttpConnect {
        host: String,
        port: u16,
        #[serde(default)]
        vault_entry_id: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyResource {
    pub id: Uuid,
    pub name: String,
    pub config: ProxyConfig,
}
impl ProxyResource {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        match &self.config {
            ProxyConfig::SshJump { .. } => Ok(()),
            ProxyConfig::Socks5 {
                host,
                port,
                vault_entry_id,
            }
            | ProxyConfig::HttpConnect {
                host,
                port,
                vault_entry_id,
            } => {
                validate_endpoint(host, *port)?;
                if vault_entry_id
                    .as_ref()
                    .is_some_and(|id| id.trim().is_empty())
                {
                    return Err(NetworkError::Invalid("empty Vault reference".into()));
                }
                Ok(())
            }
        }
    }
}
fn loopback() -> String {
    "127.0.0.1".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardingRule {
    pub id: Uuid,
    pub name: String,
    pub profile_id: Uuid,
    pub kind: TunnelKind,
    #[serde(default = "loopback")]
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
}
impl ForwardingRule {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        self.bind_host
            .parse::<std::net::IpAddr>()
            .map_err(|_| NetworkError::Invalid("bind_host must be an IP address".into()))?;
        if self.kind != TunnelKind::Dynamic {
            validate_endpoint(&self.target_host, self.target_port)?;
        }
        Ok(())
    }
}
fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 256 {
        Err(NetworkError::Invalid(
            "name required (max 256 bytes)".into(),
        ))
    } else {
        Ok(())
    }
}
pub fn validate_endpoint(host: &str, port: u16) -> Result<()> {
    if host.is_empty()
        || host.len() > 255
        || host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '/' | '@' | '[' | ']'))
        || port == 0
    {
        Err(NetworkError::Invalid(
            "valid host and nonzero port required".into(),
        ))
    } else {
        Ok(())
    }
}
#[derive(Clone)]
pub struct NetworkResourceStore {
    db: sled::Db,
    lock: Arc<Mutex<()>>,
}
impl NetworkResourceStore {
    /// Pass the application data directory. Never use the settings DB or export via Appearance.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db: sled::open(dir.as_ref().join("network-resources.sled"))?,
            lock: Arc::new(Mutex::new(())),
        })
    }
    fn list<T: serde::de::DeserializeOwned>(&self, tree: &str) -> Result<Vec<T>> {
        self.db
            .open_tree(tree)?
            .iter()
            .map(|v| {
                let (_, v) = v?;
                Ok(serde_json::from_slice(&v)?)
            })
            .collect()
    }
    pub fn list_proxies(&self) -> Result<Vec<ProxyResource>> {
        self.list("proxies")
    }
    pub fn list_rules(&self) -> Result<Vec<ForwardingRule>> {
        self.list("forwarding")
    }
    pub fn get_proxy(&self, id: Uuid) -> Result<Option<ProxyResource>> {
        self.get("proxies", id)
    }
    pub fn get_rule(&self, id: Uuid) -> Result<Option<ForwardingRule>> {
        self.get("forwarding", id)
    }
    fn get<T: serde::de::DeserializeOwned>(&self, tree: &str, id: Uuid) -> Result<Option<T>> {
        self.db
            .open_tree(tree)?
            .get(id.as_bytes())?
            .map(|v| serde_json::from_slice(&v).map_err(Into::into))
            .transpose()
    }
    fn save<T: Serialize>(&self, tree: &str, id: Uuid, value: &T) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| NetworkError::Storage("lock poisoned".into()))?;
        let tree = self.db.open_tree(tree)?;
        tree.insert(id.as_bytes(), serde_json::to_vec(value)?)?;
        tree.flush()?;
        Ok(())
    }
    pub fn save_proxy(&self, proxy: ProxyResource) -> Result<()> {
        proxy.validate()?;
        self.save("proxies", proxy.id, &proxy)
    }
    /// Caller validates profile_id exists and is SSH under the shared resource mutation lock.
    pub fn save_rule(&self, rule: ForwardingRule) -> Result<()> {
        rule.validate()?;
        self.save("forwarding", rule.id, &rule)
    }
    fn delete(&self, tree: &str, id: Uuid, dependents: &[String]) -> Result<bool> {
        if !dependents.is_empty() {
            return Err(NetworkError::Referenced(dependents.join(", ")));
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| NetworkError::Storage("lock poisoned".into()))?;
        let tree = self.db.open_tree(tree)?;
        let removed = tree.remove(id.as_bytes())?.is_some();
        tree.flush()?;
        Ok(removed)
    }
    /// Caller supplies all profile references to this proxy (never names as identities).
    pub fn delete_proxy(&self, id: Uuid, dependents: &[String]) -> Result<bool> {
        self.delete("proxies", id, dependents)
    }
    /// Running rule IDs must be supplied as dependencies. Stop first; never silently stop on delete.
    pub fn delete_rule(&self, id: Uuid, dependents: &[String]) -> Result<bool> {
        self.delete("forwarding", id, dependents)
    }
    pub fn profile_dependents(&self, id: Uuid) -> Result<Vec<String>> {
        let mut deps: Vec<String> = self
            .list_rules()?
            .into_iter()
            .filter(|r| r.profile_id == id)
            .map(|r| format!("forwarding:{}:{}", r.id, r.name))
            .collect();
        deps.extend(
            self.list_proxies()?
                .into_iter()
                .filter(|p| matches!(p.config,ProxyConfig::SshJump {profile_id} if profile_id==id))
                .map(|p| format!("proxy:{}:{}", p.id, p.name)),
        );
        Ok(deps)
    }
    pub fn vault_dependents(&self, id: &str) -> Result<Vec<String>> {
        Ok(self
            .list_proxies()?
            .into_iter()
            .filter(|p| match &p.config {
                ProxyConfig::Socks5 { vault_entry_id, .. }
                | ProxyConfig::HttpConnect { vault_entry_id, .. } => {
                    vault_entry_id.as_deref() == Some(id)
                }
                _ => false,
            })
            .map(|p| format!("proxy:{}:{}", p.id, p.name))
            .collect())
    }
}
/// Runtime ownership is in-memory only. Editing a definition never changes an existing instance.
#[derive(Default)]
pub struct ForwardingRuntime {
    running: tokio::sync::Mutex<HashMap<Uuid, Uuid>>,
}
impl ForwardingRuntime {
    /// Resolve rule.profile_id using latest profile/Vault/proxy refs before invoking.
    pub async fn start(
        &self,
        rule: &ForwardingRule,
        profile: SshProfile,
        tunnels: &TunnelManager,
        known_hosts: Option<KnownHosts>,
    ) -> Result<TunnelMeta> {
        rule.validate()?;
        let mut running = self.running.lock().await;
        if running.contains_key(&rule.id) {
            return Err(NetworkError::Invalid("rule is already running".into()));
        }
        let meta = tunnels
            .open(
                TunnelOpenRequest {
                    profile,
                    kind: rule.kind,
                    bind_host: rule.bind_host.clone(),
                    bind_port: rule.bind_port,
                    target_host: rule.target_host.clone(),
                    target_port: rule.target_port,
                },
                known_hosts,
            )
            .await
            .map_err(|e| NetworkError::Invalid(e.to_string()))?;
        running.insert(rule.id, meta.id);
        Ok(meta)
    }
    pub async fn stop(&self, id: Uuid, tunnels: &TunnelManager) -> bool {
        let mut running = self.running.lock().await;
        if let Some(tunnel) = running.remove(&id) {
            tunnels.close(tunnel).await
        } else {
            false
        }
    }
    pub async fn running(&self, tunnels: &TunnelManager) -> Vec<ForwardingInstance> {
        let running = self.running.lock().await;
        let metas = tunnels.list().await;
        running
            .iter()
            .filter_map(|(rule_id, tunnel_id)| {
                metas
                    .iter()
                    .find(|m| m.id == *tunnel_id)
                    .map(|meta| ForwardingInstance {
                        rule_id: *rule_id,
                        tunnel: meta.clone(),
                    })
            })
            .collect()
    }
    pub async fn dependents(&self, id: Uuid) -> Vec<String> {
        if self.running.lock().await.contains_key(&id) {
            vec!["running forwarding instance".into()]
        } else {
            vec![]
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ForwardingInstance {
    pub rule_id: Uuid,
    pub tunnel: TunnelMeta,
}
