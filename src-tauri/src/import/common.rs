use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::profile::{Profile, ProfileKind};
use crate::ssh::AuthMethod;

use super::types::{ImportCandidate, ImportCandidateStatus};

pub fn read_text_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

pub fn strip_utf8_bom(text: String) -> String {
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn build_profile(
    name: String,
    group: Option<String>,
    tags: Vec<String>,
    note: Option<String>,
    spec: ProfileKind,
) -> Profile {
    Profile {
        schema_version: 1,
        id: Uuid::new_v4(),
        name,
        group,
        tags,
        note,
        icon: None,
        favorite: false,
        spec,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn ready_candidate(
    source: &str,
    source_id: String,
    name: String,
    group: Option<String>,
    tags: Vec<String>,
    note: Option<String>,
    kind: &str,
    profile: Profile,
    warnings: Vec<String>,
) -> ImportCandidate {
    ImportCandidate {
        source_id,
        source: source.into(),
        name,
        group,
        tags,
        note,
        kind: kind.into(),
        status: ImportCandidateStatus::Ready,
        warnings,
        error_message: None,
        duplicate_of: None,
        profile: Some(profile),
    }
}

pub fn error_candidate(
    source: &str,
    source_id: String,
    name: String,
    message: String,
    tags: Vec<String>,
) -> ImportCandidate {
    ImportCandidate {
        source_id,
        source: source.into(),
        name,
        group: None,
        tags,
        note: None,
        kind: "error".into(),
        status: ImportCandidateStatus::Error,
        warnings: Vec::new(),
        error_message: Some(message),
        duplicate_of: None,
        profile: None,
    }
}

/// Merge optional SSH user/auth from the import wizard onto a parsed profile.
pub fn apply_ssh_import_overrides(
    profile: &mut Profile,
    user: Option<&str>,
    auth: Option<&AuthMethod>,
) {
    let ProfileKind::Ssh { ssh } = &mut profile.spec else {
        return;
    };
    if let Some(u) = user.map(str::trim).filter(|s| !s.is_empty()) {
        ssh.user = u.to_string();
    }
    if let Some(a) = auth {
        ssh.auth = a.clone();
    }
}

/// When overwriting, merge import metadata onto the stored profile and force user/auth.
pub fn merge_import_overwrite(
    existing: Option<Profile>,
    import: &Profile,
    user: Option<&str>,
    auth: Option<&AuthMethod>,
) -> Profile {
    let mut out = existing.unwrap_or_else(|| import.clone());
    out.name = import.name.clone();
    out.group = import.group.clone();
    out.tags = import.tags.clone();
    out.note = import.note.clone();
    if let (ProfileKind::Ssh { ssh: src }, ProfileKind::Ssh { ssh: dst }) =
        (&import.spec, &mut out.spec)
    {
        dst.host = src.host.clone();
        dst.port = src.port;
    }
    apply_ssh_import_overrides(&mut out, user, auth);
    if auth.is_none() || user.is_none() || user.is_some_and(|u| u.trim().is_empty()) {
        if let (ProfileKind::Ssh { ssh: src }, ProfileKind::Ssh { ssh: dst }) =
            (&import.spec, &mut out.spec)
        {
            if user.is_none() || user.is_some_and(|u| u.trim().is_empty()) {
                if !src.user.is_empty() {
                    dst.user = src.user.clone();
                }
            }
            if auth.is_none() {
                dst.auth = src.auth.clone();
            }
        }
    }
    out
}

#[cfg(test)]
mod apply_tests {
    use super::*;
    use crate::profile::ProfileKind;
    use crate::ssh::{AuthMethod, SshProfile};
    use uuid::Uuid;

    fn ssh_profile(user: &str, auth: AuthMethod) -> Profile {
        Profile {
            schema_version: 1,
            id: Uuid::new_v4(),
            name: "t".into(),
            group: None,
            tags: vec![],
            note: None,
            icon: None,
            favorite: false,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "1.2.3.4".into(),
                    port: 22,
                    user: user.into(),
                    auth,
                    jump_via: vec![],
                },
            },
        }
    }

    #[test]
    fn apply_ssh_import_overrides_user_and_password() {
        let mut p = ssh_profile("root", AuthMethod::Agent);
        apply_ssh_import_overrides(
            &mut p,
            Some("deploy"),
            Some(&AuthMethod::Password {
                secret: "secret".into(),
            }),
        );
        if let ProfileKind::Ssh { ssh } = &p.spec {
            assert_eq!(ssh.user, "deploy");
            assert!(matches!(ssh.auth, AuthMethod::Password { .. }));
        } else {
            panic!("expected ssh");
        }
    }

    #[test]
    fn merge_import_overwrite_applies_vault_auth() {
        let existing_id = Uuid::new_v4();
        let existing = Profile {
            schema_version: 1,
            id: existing_id,
            name: "Old".into(),
            group: None,
            tags: vec![],
            note: None,
            icon: None,
            favorite: true,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "10.0.0.1".into(),
                    port: 22,
                    user: "root".into(),
                    auth: AuthMethod::PublicKey {
                        key_path: "/old/key".into(),
                        passphrase: None,
                    },
                    jump_via: vec![],
                },
            },
        };
        let import = ssh_profile(
            "devops",
            AuthMethod::VaultRef {
                entry_id: "vault-key".into(),
                passphrase_entry_id: None,
            },
        );
        let merged = merge_import_overwrite(
            Some(existing),
            &import,
            Some("devops"),
            Some(&AuthMethod::VaultRef {
                entry_id: "vault-key".into(),
                passphrase_entry_id: None,
            }),
        );
        assert_eq!(merged.id, existing_id);
        assert!(merged.favorite);
        if let ProfileKind::Ssh { ssh } = &merged.spec {
            assert_eq!(ssh.user, "devops");
            assert!(matches!(ssh.auth, AuthMethod::VaultRef { .. }));
        } else {
            panic!("expected ssh");
        }
    }
}
