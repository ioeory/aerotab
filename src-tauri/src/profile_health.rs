use std::time::Duration;

use russh::Disconnect;
use serde::Serialize;
use tokio::time::timeout;
use uuid::Uuid;

use crate::profile::{Profile, ProfileKind};
use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::{self, vault_resolve, AuthMethod, SshProfile};
use crate::vault::VaultStore;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileHealthCheck {
    pub name: String,
    pub status: HealthStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileHealthResult {
    pub id: Uuid,
    pub name: String,
    pub status: HealthStatus,
    pub checks: Vec<ProfileHealthCheck>,
}

pub async fn check_profiles(
    profiles: Vec<Profile>,
    known_hosts: Option<KnownHosts>,
    connect: bool,
    vault: Option<VaultStore>,
) -> Vec<ProfileHealthResult> {
    let mut results = Vec::with_capacity(profiles.len());
    for profile in profiles {
        results.push(check_profile(profile, known_hosts.clone(), connect, vault.clone()).await);
    }
    results
}

async fn check_profile(
    profile: Profile,
    known_hosts: Option<KnownHosts>,
    connect: bool,
    vault: Option<VaultStore>,
) -> ProfileHealthResult {
    let mut checks = Vec::new();
    match profile.spec {
        ProfileKind::Ssh { ssh } => {
            inspect_ssh_profile("Target", &ssh, &known_hosts, &mut checks);
            if connect {
                run_connection_check(&ssh, known_hosts.clone(), vault.as_ref(), &mut checks).await;
            }
        }
        ProfileKind::Rdp { rdp } | ProfileKind::Vnc { spec: rdp } => {
            checks.push(ProfileHealthCheck {
                name: "Remote desktop".into(),
                status: HealthStatus::Ok,
                message: format!("{}:{} (use remote.openProfile)", rdp.host, rdp.port),
            });
            if rdp.ssh_profile_id.is_some() {
                checks.push(ProfileHealthCheck {
                    name: "SSH tunnel".into(),
                    status: HealthStatus::Ok,
                    message: "Will forward via linked SSH profile".into(),
                });
            }
        }
    }
    let status = checks
        .iter()
        .map(|check| check.status)
        .max()
        .unwrap_or(HealthStatus::Ok);
    ProfileHealthResult {
        id: profile.id,
        name: profile.name,
        status,
        checks,
    }
}

fn inspect_ssh_profile(
    label: &str,
    profile: &SshProfile,
    known_hosts: &Option<KnownHosts>,
    checks: &mut Vec<ProfileHealthCheck>,
) {
    if profile.host.trim().is_empty() {
        push_check(checks, label, HealthStatus::Error, "host is empty");
    } else if profile.port == 0 {
        push_check(
            checks,
            label,
            HealthStatus::Error,
            "port must be between 1 and 65535",
        );
    } else if profile.user.trim().is_empty() {
        push_check(checks, label, HealthStatus::Error, "user is empty");
    } else {
        push_check(
            checks,
            label,
            HealthStatus::Ok,
            format!("{}@{}:{}", profile.user, profile.host, profile.port),
        );
    }

    inspect_auth(label, &profile.auth, checks);
    inspect_known_host(label, profile, known_hosts, checks);

    for (idx, jump) in profile.jump_via.iter().enumerate() {
        inspect_ssh_profile(&format!("Jump {}", idx + 1), jump, known_hosts, checks);
    }
}

fn inspect_auth(label: &str, auth: &AuthMethod, checks: &mut Vec<ProfileHealthCheck>) {
    match auth {
        AuthMethod::Password { secret } => {
            if secret.is_empty() {
                push_check(
                    checks,
                    format!("{label} auth"),
                    HealthStatus::Error,
                    "password is empty",
                );
            } else {
                push_check(
                    checks,
                    format!("{label} auth"),
                    HealthStatus::Ok,
                    "password auth configured",
                );
            }
        }
        AuthMethod::PublicKey { key_path, .. } => {
            if key_path.as_os_str().is_empty() {
                push_check(
                    checks,
                    format!("{label} key"),
                    HealthStatus::Error,
                    "private key path is empty",
                );
            } else if !key_path.exists() {
                push_check(
                    checks,
                    format!("{label} key"),
                    HealthStatus::Error,
                    format!("private key does not exist: {}", key_path.display()),
                );
            } else if std::fs::File::open(key_path).is_err() {
                push_check(
                    checks,
                    format!("{label} key"),
                    HealthStatus::Error,
                    format!("private key is not readable: {}", key_path.display()),
                );
            } else {
                push_check(
                    checks,
                    format!("{label} key"),
                    HealthStatus::Ok,
                    format!("private key is readable: {}", key_path.display()),
                );
            }
        }
        AuthMethod::Agent => {
            push_check(
                checks,
                format!("{label} auth"),
                HealthStatus::Ok,
                "agent auth configured; live availability is checked during connection probing",
            );
        }
        AuthMethod::VaultRef { entry_id, .. } => {
            if entry_id.trim().is_empty() {
                push_check(
                    checks,
                    format!("{label} vault"),
                    HealthStatus::Error,
                    "vault entry id is empty",
                );
            } else {
                push_check(
                    checks,
                    format!("{label} vault"),
                    HealthStatus::Ok,
                    format!("vault entry {entry_id} (resolved at connect time)"),
                );
            }
        }
    }
}

fn inspect_known_host(
    label: &str,
    profile: &SshProfile,
    known_hosts: &Option<KnownHosts>,
    checks: &mut Vec<ProfileHealthCheck>,
) {
    let Some(store) = known_hosts else {
        push_check(
            checks,
            format!("{label} known_hosts"),
            HealthStatus::Warning,
            "known_hosts store is not configured",
        );
        return;
    };
    let host_port = format!("{}:{}", profile.host, profile.port);
    let pinned = store.list().into_iter().any(|(host, _)| host == host_port);
    if pinned {
        push_check(
            checks,
            format!("{label} known_hosts"),
            HealthStatus::Ok,
            format!("{host_port} is pinned"),
        );
    } else {
        push_check(
            checks,
            format!("{label} known_hosts"),
            HealthStatus::Warning,
            format!("{host_port} has not been seen yet"),
        );
    }
}

async fn run_connection_check(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
    vault: Option<&VaultStore>,
    checks: &mut Vec<ProfileHealthCheck>,
) {
    let mut probe_profile = profile.clone();
    if let Err(err) = vault_resolve::resolve_profile_vault_auth(vault, &mut probe_profile).await {
        push_check(checks, "Connection", HealthStatus::Error, err.to_string());
        return;
    }
    let probe = timeout(
        Duration::from_secs(12),
        ssh::connect_authenticated(
            &probe_profile,
            known_hosts,
            ssh::SshTransportSettings::default(),
        ),
    )
    .await;
    match probe {
        Ok(Ok(handle)) => {
            let _ = handle
                .disconnect(
                    Disconnect::ByApplication,
                    "profile health check complete",
                    "en",
                )
                .await;
            push_check(
                checks,
                "Connection",
                HealthStatus::Ok,
                "authenticated successfully",
            );
        }
        Ok(Err(err)) => {
            push_check(checks, "Connection", HealthStatus::Error, err.to_string());
        }
        Err(_) => {
            push_check(
                checks,
                "Connection",
                HealthStatus::Error,
                "connection timed out after 12s",
            );
        }
    }
}

fn push_check(
    checks: &mut Vec<ProfileHealthCheck>,
    name: impl Into<String>,
    status: HealthStatus,
    message: impl Into<String>,
) {
    checks.push(ProfileHealthCheck {
        name: name.into(),
        status,
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileIcon;
    use std::path::PathBuf;

    fn profile(auth: AuthMethod) -> Profile {
        Profile {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "prod".into(),
            group: None,
            tags: vec![],
            icon: Some(ProfileIcon {
                kind: "builtin".into(),
                value: "server".into(),
            }),
            favorite: false,
            note: None,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "example.com".into(),
                    port: 22,
                    user: "root".into(),
                    auth,
                    jump_via: vec![],
                },
            },
        }
    }

    #[tokio::test]
    async fn flags_missing_public_key() {
        let result = check_profile(
            profile(AuthMethod::PublicKey {
                key_path: PathBuf::from("/definitely/missing/tabby/key"),
                passphrase: None,
            }),
            None,
            false,
            None,
        )
        .await;
        assert_eq!(result.status, HealthStatus::Error);
        assert!(result
            .checks
            .iter()
            .any(|check| check.message.contains("does not exist")));
    }

    #[tokio::test]
    async fn accepts_agent_auth_configuration() {
        let result = check_profile(profile(AuthMethod::Agent), None, false, None).await;
        assert_eq!(result.status, HealthStatus::Warning);
        assert!(result
            .checks
            .iter()
            .any(|check| check.status == HealthStatus::Ok && check.message.contains("agent auth")));
    }
}
