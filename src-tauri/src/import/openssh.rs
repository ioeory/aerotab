use std::path::{Path, PathBuf};

use crate::profile::ProfileKind;
use crate::ssh::{AuthMethod, SshProfile};
use crate::ssh_config::{self, SshConfigEntry};

use super::common::{build_profile, error_candidate, read_text_file, ready_candidate};
use super::types::{ImportCandidate, ImportDetectPath, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "ssh-config";
const IMPORT_TAG: &str = "import:ssh-config";

pub fn detect_openssh_paths() -> ImportDetectResult {
    let mut paths = Vec::new();
    if let Some(path) = ssh_config::default_config_path() {
        if path.is_file() {
            paths.push(ImportDetectPath {
                path: path.display().to_string(),
                label: "~/.ssh/config".into(),
            });
        }
    }
    ImportDetectResult { paths }
}

pub fn read_openssh_file(path: &Path) -> Result<String, String> {
    read_text_file(path)
}

pub fn preview_openssh(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let entries = ssh_config::parse(text);
    if entries.is_empty() {
        return Err("no importable Host entries found in ssh config".into());
    }
    let candidates = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| map_entry(entry, idx, &entries))
        .collect::<Vec<_>>();
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn map_entry(entry: &SshConfigEntry, idx: usize, catalog: &[SshConfigEntry]) -> ImportCandidate {
    let source_id = format!("{}:{}:{}", entry.alias, entry.host, entry.port);
    let name = entry.alias.clone();
    let tags = vec![IMPORT_TAG.to_string()];
    let note = Some("Imported from OpenSSH config".into());

    if entry.host.is_empty() {
        return error_candidate(SOURCE, source_id, name, "missing HostName".into(), tags);
    }

    let mut warnings = Vec::new();
    if entry.user.is_none() {
        warnings.push("no User directive; defaulting to root".into());
    }
    let jump_via = jump_via_from_config(entry, catalog, &mut warnings);
    let auth = auth_for_entry(entry);
    let ssh = SshProfile {
        host: entry.host.clone(),
        port: entry.port,
        user: entry.user.clone().unwrap_or_else(|| "root".into()),
        auth,
        jump_via,
    };
    let profile = build_profile(
        name.clone(),
        None,
        tags.clone(),
        note,
        ProfileKind::Ssh { ssh },
    );
    ready_candidate(
        SOURCE,
        source_id,
        name,
        None,
        tags,
        Some(format!("Host alias from ssh config (row {})", idx + 1)),
        "ssh",
        profile,
        warnings,
    )
}

fn auth_for_entry(entry: &SshConfigEntry) -> AuthMethod {
    if let Some(path) = entry.identity_file.as_ref() {
        AuthMethod::PublicKey {
            key_path: PathBuf::from(path),
            passphrase: None,
        }
    } else {
        AuthMethod::Agent
    }
}

fn jump_via_from_config(
    entry: &SshConfigEntry,
    catalog: &[SshConfigEntry],
    warnings: &mut Vec<String>,
) -> Vec<SshProfile> {
    let mut hops = Vec::new();
    for token in &entry.proxy_jump {
        let Some(hop) = catalog
            .iter()
            .find(|e| e.alias == *token || e.host == *token)
        else {
            warnings.push(format!("ProxyJump host '{token}' not found in config"));
            continue;
        };
        hops.push(SshProfile {
            host: hop.host.clone(),
            port: hop.port,
            user: hop.user.clone().unwrap_or_else(|| "root".into()),
            auth: auth_for_entry(hop),
            jump_via: Vec::new(),
        });
    }
    hops
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
        Host my-server
          HostName 1.2.3.4
          User alice
          Port 2222
          IdentityFile ~/.ssh/id_ed25519

        Host via-bastion
          HostName 10.0.0.2
          User deploy
          ProxyJump bastion

        Host bastion
          HostName 10.0.0.1
          User jump
    ";

    #[test]
    fn maps_hosts_with_jump_and_key() {
        let preview = preview_openssh(SAMPLE, None).expect("preview");
        assert_eq!(preview.candidates.len(), 3);
        let server = preview
            .candidates
            .iter()
            .find(|c| c.name == "my-server")
            .expect("my-server");
        assert_eq!(server.kind, "ssh");
        assert_eq!(
            server.status,
            super::super::types::ImportCandidateStatus::Ready
        );
        let profile = server.profile.as_ref().expect("profile");
        match &profile.spec {
            ProfileKind::Ssh { ssh } => {
                assert_eq!(ssh.host, "1.2.3.4");
                assert_eq!(ssh.port, 2222);
                assert_eq!(ssh.user, "alice");
                assert!(matches!(ssh.auth, AuthMethod::PublicKey { .. }));
            }
            _ => panic!("expected ssh"),
        }

        let via = preview
            .candidates
            .iter()
            .find(|c| c.name == "via-bastion")
            .expect("via-bastion");
        let ProfileKind::Ssh { ssh } = &via.profile.as_ref().unwrap().spec else {
            panic!("ssh");
        };
        assert_eq!(ssh.jump_via.len(), 1);
        assert_eq!(ssh.jump_via[0].host, "10.0.0.1");
    }
}
