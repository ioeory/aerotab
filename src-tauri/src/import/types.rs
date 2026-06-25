use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::profile::Profile;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDetectPath {
    pub path: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDetectResult {
    pub paths: Vec<ImportDetectPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportCandidateStatus {
    Ready,
    Duplicate,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub source_id: String,
    pub source: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub kind: String,
    pub status: ImportCandidateStatus,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewResult {
    pub source: String,
    pub path: Option<String>,
    pub candidates: Vec<ImportCandidate>,
    pub stats: ImportPreviewStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewStats {
    pub total: usize,
    pub ready: usize,
    pub duplicate: usize,
    pub error: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportApplyResult {
    pub created: usize,
    pub skipped: usize,
    pub updated: usize,
    pub errors: Vec<String>,
}

pub fn endpoint_key(profile: &Profile) -> Option<String> {
    use crate::profile::ProfileKind;
    match &profile.spec {
        ProfileKind::Ssh { ssh } => Some(format!(
            "ssh:{}@{}:{}",
            ssh.user.to_ascii_lowercase(),
            ssh.host.to_ascii_lowercase(),
            ssh.port
        )),
        ProfileKind::Rdp { rdp } => Some(format!(
            "rdp:{}:{}",
            rdp.host.to_ascii_lowercase(),
            rdp.port
        )),
        ProfileKind::Vnc { spec } => Some(format!(
            "vnc:{}:{}",
            spec.host.to_ascii_lowercase(),
            spec.port
        )),
    }
}

fn host_port_key(profile: &Profile) -> Option<String> {
    use crate::profile::ProfileKind;
    match &profile.spec {
        ProfileKind::Ssh { ssh } => Some(format!("{}:{}", ssh.host.to_ascii_lowercase(), ssh.port)),
        ProfileKind::Rdp { rdp } => Some(format!("{}:{}", rdp.host.to_ascii_lowercase(), rdp.port)),
        ProfileKind::Vnc { spec } => {
            Some(format!("{}:{}", spec.host.to_ascii_lowercase(), spec.port))
        }
    }
}

pub fn mark_duplicates(candidates: &mut [ImportCandidate], existing: &[Profile]) {
    let mut seen: std::collections::HashMap<String, Uuid> = std::collections::HashMap::new();
    let mut seen_host_port: std::collections::HashMap<String, Uuid> =
        std::collections::HashMap::new();
    for p in existing {
        if let Some(k) = endpoint_key(p) {
            seen.entry(k).or_insert(p.id);
        }
        if let Some(k) = host_port_key(p) {
            seen_host_port.entry(k).or_insert(p.id);
        }
    }
    for c in candidates.iter_mut() {
        let Some(profile) = c.profile.as_ref() else {
            continue;
        };
        if c.status != ImportCandidateStatus::Ready && c.status != ImportCandidateStatus::Duplicate
        {
            continue;
        }
        let hit = endpoint_key(profile)
            .and_then(|key| seen.get(&key).copied())
            .or_else(|| host_port_key(profile).and_then(|key| seen_host_port.get(&key).copied()));
        if let Some(id) = hit {
            c.status = ImportCandidateStatus::Duplicate;
            c.duplicate_of = Some(id);
        } else if c.status == ImportCandidateStatus::Duplicate {
            c.status = ImportCandidateStatus::Ready;
            c.duplicate_of = None;
        }
    }
}

/// Find a stored profile id matching the same endpoint as `profile`.
pub fn existing_id_for_endpoint(existing: &[Profile], profile: &Profile) -> Option<Uuid> {
    let key = endpoint_key(profile)?;
    if let Some(id) = existing
        .iter()
        .find_map(|p| endpoint_key(p).filter(|k| k == &key).map(|_| p.id))
    {
        return Some(id);
    }
    let hp = host_port_key(profile)?;
    existing
        .iter()
        .find_map(|p| host_port_key(p).filter(|k| k == &hp).map(|_| p.id))
}

pub fn preview_stats(candidates: &[ImportCandidate]) -> ImportPreviewStats {
    let mut ready = 0;
    let mut duplicate = 0;
    let mut error = 0;
    for c in candidates {
        match c.status {
            ImportCandidateStatus::Ready => ready += 1,
            ImportCandidateStatus::Duplicate => duplicate += 1,
            ImportCandidateStatus::Error => error += 1,
        }
    }
    ImportPreviewStats {
        total: candidates.len(),
        ready,
        duplicate,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Profile, ProfileKind};
    use crate::ssh::{AuthMethod, SshProfile};

    fn ssh(user: &str, host: &str) -> Profile {
        Profile {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: host.into(),
            group: None,
            tags: vec![],
            note: None,
            icon: None,
            favorite: false,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: host.into(),
                    port: 22,
                    user: user.into(),
                    auth: AuthMethod::Agent,
                    jump_via: vec![],
                },
            },
        }
    }

    #[test]
    fn existing_id_for_endpoint_matches_case_insensitive() {
        let id = Uuid::new_v4();
        let mut existing = ssh("Root", "HOST.example");
        existing.id = id;
        let probe = ssh("root", "host.example");
        assert_eq!(
            existing_id_for_endpoint(std::slice::from_ref(&existing), &probe),
            Some(id)
        );
    }

    #[test]
    fn existing_id_for_endpoint_falls_back_to_host_port() {
        let id = Uuid::new_v4();
        let mut existing = ssh("root", "10.0.0.5");
        existing.id = id;
        let probe = ssh("devops", "10.0.0.5");
        assert_eq!(
            existing_id_for_endpoint(std::slice::from_ref(&existing), &probe),
            Some(id)
        );
    }
}
