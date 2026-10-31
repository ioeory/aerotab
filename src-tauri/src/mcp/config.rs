//! Device-local MCP listener and authorization settings. Secrets never live here.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::IpAddr};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConfig {
    pub enabled: bool,
    pub bind_host: String,
    pub allow_remote_bind: bool,
    pub port: u16,
    pub additional_context: String,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub local_roots: Vec<String>,
    pub permissions: HashMap<String, bool>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_host: "127.0.0.1".into(),
            allow_remote_bind: false,
            port: 23890,
            additional_context: String::new(),
            tls_cert_path: None,
            tls_key_path: None,
            local_roots: Vec::new(),
            permissions: HashMap::from([
                ("listProfiles".into(), true),
                ("readTerminal".into(), true),
                ("writeTerminal".into(), false),
                ("executeCommand".into(), false),
                ("manageSessions".into(), false),
                ("sftp".into(), false),
                ("localFiles".into(), false),
                ("tunnels".into(), false),
                ("callRpc".into(), false),
            ]),
        }
    }
}

impl McpConfig {
    pub fn permitted(&self, permission: &str) -> bool {
        self.permissions.get(permission).copied().unwrap_or(false)
    }

    pub fn bind_ip(&self) -> Result<IpAddr, String> {
        if self.bind_host == "localhost" {
            return Ok(IpAddr::from([127, 0, 0, 1]));
        }
        self.bind_host
            .parse()
            .map_err(|_| "bindHost must be localhost or a literal IP address".into())
    }

    pub fn tls_enabled(&self) -> bool {
        self.tls_cert_path
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && self
                .tls_key_path
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
    }

    pub fn validate(&self) -> Result<(), String> {
        let ip = self.bind_ip()?;
        if self.port == 0 {
            return Err("port must be between 1 and 65535".into());
        }
        let cert = self
            .tls_cert_path
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        let key = self
            .tls_key_path
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());
        if cert != key {
            return Err("TLS requires both tlsCertPath and tlsKeyPath".into());
        }
        if !ip.is_loopback() && (!self.allow_remote_bind || !self.tls_enabled()) {
            return Err(
                "Remote MCP bind requires allowRemoteBind and TLS certificate + key".into(),
            );
        }
        if self.additional_context.len() > 65536 {
            return Err("additionalContext exceeds 64 KiB".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_requires_opt_in_and_tls() {
        let mut cfg = McpConfig {
            bind_host: "0.0.0.0".into(),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
        cfg.allow_remote_bind = true;
        assert!(cfg.validate().is_err());
        cfg.tls_cert_path = Some("certificate.pem".into());
        cfg.tls_key_path = Some("private-key.pem".into());
        assert!(cfg.validate().is_ok());
    }
    #[test]
    fn defaults_are_safe_and_camel_case() {
        let cfg: McpConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.validate().is_ok());
        let value = serde_json::to_value(cfg).unwrap();
        assert_eq!(value["bindHost"], "127.0.0.1");
        assert!(value.get("apiKey").is_none());
    }
}
