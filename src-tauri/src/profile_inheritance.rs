//! Explicit, field-level SSH inheritance. Inline SSH snapshots never imply identity.
use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    network_resources::{ProxyConfig, ProxyResource},
    profile::{Profile, ProfileKind},
    ssh::AuthMethod,
};

pub const SETTINGS_KEY: &str = "profile.groupDefaults";
pub type GroupDefaults = BTreeMap<String, BTreeMap<String, Value>>;

#[derive(Debug, Serialize)]
pub struct EffectiveProfile {
    pub profile: Profile,
    pub sources: BTreeMap<String, String>,
}

pub fn normalize_group(path: &str) -> String {
    path.split(['/', '\\', '>'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn parse_defaults(value: &Value) -> Result<GroupDefaults, String> {
    let object = value
        .as_object()
        .ok_or("group defaults must be an object")?;
    let mut result = GroupDefaults::new();
    for (path, raw) in object {
        if path.is_empty() || normalize_group(path) != *path || path.chars().any(char::is_control) {
            return Err("group defaults require normalized nonempty paths".into());
        }
        let fields = raw
            .as_object()
            .ok_or("group defaults entry must be an object")?;
        for (field, value) in fields {
            let valid = match field.as_str() {
                "user" => value
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && !s.chars().any(char::is_control)),
                "port" => value
                    .as_u64()
                    .is_some_and(|p| p > 0 && p <= u16::MAX as u64),
                "proxy_id" => {
                    value.is_null() || serde_json::from_value::<Uuid>(value.clone()).is_ok()
                }
                "jump_profile_ids" => serde_json::from_value::<Vec<Uuid>>(value.clone()).is_ok(),
                "auth" => {
                    // Validate the exact wire shape too: serde normally ignores unknown fields.
                    match value {
                        Value::String(s) => s == "Agent",
                        Value::Object(auth) if auth.len() == 1 => {
                            auth.iter().next().is_some_and(|(kind, payload)| {
                                let Some(fields) = payload.as_object() else {
                                    return false;
                                };
                                match kind.as_str() {
                                    "PublicKey" => {
                                        fields.keys().all(|k| k == "key_path" || k == "passphrase")
                                            && fields
                                                .get("key_path")
                                                .and_then(Value::as_str)
                                                .is_some_and(|s| !s.trim().is_empty())
                                            && fields.get("passphrase").map_or(true, Value::is_null)
                                    }
                                    "VaultRef" => {
                                        fields
                                            .keys()
                                            .all(|k| k == "entry_id" || k == "passphrase_entry_id")
                                            && fields
                                                .get("entry_id")
                                                .and_then(Value::as_str)
                                                .is_some_and(|s| !s.trim().is_empty())
                                            && fields.get("passphrase_entry_id").map_or(true, |v| {
                                                v.is_null()
                                                    || v.as_str()
                                                        .is_some_and(|s| !s.trim().is_empty())
                                            })
                                    }
                                    _ => false,
                                }
                            }) && serde_json::from_value::<AuthMethod>(value.clone()).is_ok()
                        }
                        _ => false,
                    }
                }
                _ => false,
            };
            if !valid {
                return Err(format!(
                    "invalid group default field {field}; plaintext credentials are forbidden"
                ));
            }
        }
        result.insert(
            path.clone(),
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        );
    }
    Ok(result)
}

pub fn effective_profile(
    profile: &Profile,
    defaults: &GroupDefaults,
) -> Result<EffectiveProfile, String> {
    if let ProfileKind::Ssh { ssh } = &profile.spec {
        validate_inherit(ssh)?;
    }
    merge_profile(profile, defaults)
}

fn validate_inherit(ssh: &crate::ssh::SshProfile) -> Result<(), String> {
    if ssh
        .inherit
        .iter()
        .any(|f| !["user", "port", "auth", "proxy_id", "jump"].contains(&f.as_str()))
    {
        return Err("unknown SSH inheritance field".into());
    }
    Ok(())
}

fn merge_profile(profile: &Profile, defaults: &GroupDefaults) -> Result<EffectiveProfile, String> {
    let mut result = EffectiveProfile {
        profile: profile.clone(),
        sources: BTreeMap::new(),
    };
    let ProfileKind::Ssh { ssh } = &mut result.profile.spec else {
        return Ok(result);
    };
    let group = normalize_group(profile.group.as_deref().unwrap_or(""));
    let mut path = String::new();
    for segment in group.split('/').filter(|s| !s.is_empty()) {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(segment);
        let Some(fields) = defaults.get(&path) else {
            continue;
        };
        for field in &ssh.inherit.clone() {
            let key = if field == "jump" {
                "jump_profile_ids"
            } else {
                field.as_str()
            };
            let Some(value) = fields.get(key) else {
                continue;
            };
            match field.as_str() {
                "user" => {
                    ssh.user = serde_json::from_value(value.clone()).map_err(|_| "invalid user")?
                }
                "port" => {
                    ssh.port = serde_json::from_value(value.clone()).map_err(|_| "invalid port")?
                }
                "auth" => {
                    ssh.auth = serde_json::from_value(value.clone()).map_err(|_| "invalid auth")?
                }
                "proxy_id" => {
                    ssh.proxy_id =
                        serde_json::from_value(value.clone()).map_err(|_| "invalid proxy")?
                }
                "jump" => {
                    ssh.jump_via.clear();
                    ssh.jump_profile_ids =
                        serde_json::from_value(value.clone()).map_err(|_| "invalid jump")?;
                }
                _ => continue,
            }
            result.sources.insert(field.clone(), path.clone());
        }
    }
    Ok(result)
}

/// Merge only. Unknown inheritance fields are rejected only when their node is used.
pub fn merge_catalog(
    profiles: &[Profile],
    defaults: &GroupDefaults,
) -> Result<Vec<Profile>, String> {
    profiles
        .iter()
        .map(|p| merge_profile(p, defaults).map(|p| p.profile))
        .collect()
}

/// Validate only supplied (new or changed) group entries, including unused entries.
pub fn validate_default_references(
    defaults: &GroupDefaults,
    catalog: &[Profile],
    proxies: &[ProxyResource],
) -> Result<(), String> {
    for fields in defaults.values() {
        if let Some(ids) = fields.get("jump_profile_ids") {
            for id in
                serde_json::from_value::<Vec<Uuid>>(ids.clone()).map_err(|_| "invalid jumps")?
            {
                let target = catalog
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or("group jump reference is missing or not SSH")?;
                let ProfileKind::Ssh { ssh } = &target.spec else {
                    return Err("group jump reference is missing or not SSH".into());
                };
                validate_ssh_graph(ssh, catalog, proxies, Some(id))?;
            }
        }
        if let Some(id) = fields.get("proxy_id").filter(|v| !v.is_null()) {
            let id: Uuid = serde_json::from_value(id.clone()).map_err(|_| "invalid proxy")?;
            let proxy = proxies
                .iter()
                .find(|p| p.id == id)
                .ok_or("group proxy reference is missing")?;
            if let ProxyConfig::SshJump { profile_id } = proxy.config {
                let target = catalog
                    .iter()
                    .find(|p| p.id == profile_id)
                    .ok_or("group proxy SSH profile is missing")?;
                let ProfileKind::Ssh { ssh } = &target.spec else {
                    return Err("group proxy target is not SSH".into());
                };
                validate_ssh_graph(ssh, catalog, proxies, Some(profile_id))?;
            }
        }
    }
    Ok(())
}

pub fn validate_ssh_graph(
    ssh: &crate::ssh::SshProfile,
    catalog: &[Profile],
    proxies: &[ProxyResource],
    root: Option<Uuid>,
) -> Result<(), String> {
    fn visit(
        ssh: &crate::ssh::SshProfile,
        catalog: &[Profile],
        proxies: &[ProxyResource],
        path: &mut HashSet<Uuid>,
        depth: usize,
    ) -> Result<(), String> {
        validate_inherit(ssh)?;
        if depth > 32 {
            return Err("SSH reference depth exceeds 32".into());
        }
        if !ssh.jump_via.is_empty() && !ssh.jump_profile_ids.is_empty() {
            return Err("jump representations are mutually exclusive".into());
        }
        let mut refs = ssh.jump_profile_ids.clone();
        if let Some(id) = ssh.proxy_id {
            let proxy = proxies
                .iter()
                .find(|p| p.id == id)
                .ok_or("proxy resource not found")?;
            if let ProxyConfig::SshJump { profile_id } = proxy.config {
                if !ssh.jump_via.is_empty() || !refs.is_empty() {
                    return Err("SSH proxy and jump chain cannot be combined".into());
                }
                refs.push(profile_id);
            }
        }
        for id in refs {
            if !path.insert(id) {
                return Err("SSH reference cycle".into());
            }
            let target = catalog
                .iter()
                .find(|p| p.id == id)
                .ok_or("SSH profile reference not found")?;
            let ProfileKind::Ssh { ssh } = &target.spec else {
                return Err("reference is not SSH".into());
            };
            visit(ssh, catalog, proxies, path, depth + 1)?;
            path.remove(&id);
        }
        for hop in &ssh.jump_via {
            visit(hop, catalog, proxies, path, depth + 1)?;
        }
        Ok(())
    }
    visit(ssh, catalog, proxies, &mut root.into_iter().collect(), 0)
}

/// Validate changed roots and their transitive reverse dependencies, not unrelated nodes.
pub fn validate_affected(
    catalog: &[Profile],
    proxies: &[ProxyResource],
    mut affected: HashSet<Uuid>,
) -> Result<(), String> {
    fn references(
        ssh: &crate::ssh::SshProfile,
        ids: &HashSet<Uuid>,
        proxies: &[ProxyResource],
    ) -> bool {
        ssh.jump_profile_ids.iter().any(|id| ids.contains(id))
            || ssh.proxy_id.is_some_and(|id| proxies.iter().any(|p| p.id == id && matches!(p.config, ProxyConfig::SshJump { profile_id } if ids.contains(&profile_id))))
            || ssh.jump_via.iter().any(|hop| references(hop, ids, proxies))
    }
    loop {
        let before = affected.len();
        for p in catalog {
            if matches!(&p.spec, ProfileKind::Ssh { ssh } if references(ssh, &affected, proxies)) {
                affected.insert(p.id);
            }
        }
        if before == affected.len() {
            break;
        }
    }
    for p in catalog.iter().filter(|p| affected.contains(&p.id)) {
        if let ProfileKind::Ssh { ssh } = &p.spec {
            validate_ssh_graph(ssh, catalog, proxies, Some(p.id))?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn effective_catalog(
    profiles: &[Profile],
    defaults: &GroupDefaults,
    proxies: &[ProxyResource],
) -> Result<Vec<Profile>, String> {
    let catalog = merge_catalog(profiles, defaults)?;
    validate_default_references(defaults, &catalog, proxies)?;
    validate_affected(&catalog, proxies, profiles.iter().map(|p| p.id).collect())?;
    Ok(catalog)
}

pub fn proxy_dependents_ids(profiles: &[Profile], id: Uuid) -> HashSet<Uuid> {
    fn uses(ssh: &crate::ssh::SshProfile, id: Uuid) -> bool {
        ssh.proxy_id == Some(id) || ssh.jump_via.iter().any(|hop| uses(hop, id))
    }
    profiles
        .iter()
        .filter(|p| matches!(&p.spec, ProfileKind::Ssh { ssh } if uses(ssh, id)))
        .map(|p| p.id)
        .collect()
}

pub fn proxy_dependents(profiles: &[Profile], id: Uuid) -> Vec<String> {
    let ids = proxy_dependents_ids(profiles, id);
    profiles
        .iter()
        .filter(|p| ids.contains(&p.id))
        .map(|p| p.name.clone())
        .collect()
}

pub fn group_dependents(defaults: &GroupDefaults, field: &str, id: &str) -> Vec<String> {
    defaults
        .iter()
        .filter(|(_, fields)| match field {
            "jump" => fields
                .get("jump_profile_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|v| v.as_str() == Some(id))),
            "proxy_id" => fields.get("proxy_id").and_then(Value::as_str) == Some(id),
            "auth" => fields
                .get("auth")
                .and_then(|v| v.get("VaultRef"))
                .is_some_and(|v| {
                    v.get("entry_id").and_then(Value::as_str) == Some(id)
                        || v.get("passphrase_entry_id").and_then(Value::as_str) == Some(id)
                }),
            _ => false,
        })
        .map(|(path, _)| format!("group:{path}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(group: &str, inherit: Vec<&str>) -> Profile {
        serde_json::from_value(json!({"id":Uuid::new_v4(),"name":"target","group":group,"kind":"ssh","ssh":{"host":"host","user":"local","port":22,"auth":"Agent","inherit":inherit}})).unwrap()
    }

    #[test]
    fn hierarchical_merge_preserves_local_fields_and_reports_sources() {
        let defaults =
            parse_defaults(&json!({"a":{"user":"parent","port":2222},"a/b":{"user":"child"}}))
                .unwrap();
        let effective =
            effective_profile(&profile("a/b/c", vec!["user", "port"]), &defaults).unwrap();
        let ProfileKind::Ssh { ssh } = effective.profile.spec else {
            panic!()
        };
        assert_eq!(ssh.user, "child");
        assert_eq!(ssh.port, 2222);
        assert_eq!(effective.sources["user"], "a/b");
        assert_eq!(effective.sources["port"], "a");
        let legacy = effective_profile(&profile("a/b", vec![]), &defaults).unwrap();
        let ProfileKind::Ssh { ssh } = legacy.profile.spec else {
            panic!()
        };
        assert_eq!(ssh.user, "local");
        assert!(legacy.sources.is_empty());
    }

    #[test]
    fn explicit_direct_and_missing_values_are_distinct() {
        let defaults = parse_defaults(&json!({"a":{"proxy_id":Uuid::new_v4(),"jump_profile_ids":[Uuid::new_v4()]},"a/b":{"proxy_id":null,"jump_profile_ids":[]}})).unwrap();
        let effective =
            effective_profile(&profile("a/b", vec!["proxy_id", "jump", "user"]), &defaults)
                .unwrap();
        let ProfileKind::Ssh { ssh } = effective.profile.spec else {
            panic!()
        };
        assert!(ssh.proxy_id.is_none());
        assert!(ssh.jump_profile_ids.is_empty());
        assert!(ssh.jump_via.is_empty());
        assert_eq!(ssh.user, "local");
        assert!(!effective.sources.contains_key("user"));
    }

    #[test]
    fn inherited_cycles_and_unknown_fields_fail_closed() {
        let a = profile("a", vec!["jump"]);
        let defaults = parse_defaults(&json!({"a":{"jump_profile_ids":[a.id]}})).unwrap();
        assert!(effective_catalog(&[a], &defaults, &[]).is_err());
        assert!(effective_profile(&profile("a", vec!["typo"]), &GroupDefaults::new()).is_err());
    }
}
