//! Apply selected import wizard items to the profile store.

use std::collections::HashMap;

use uuid::Uuid;

use crate::profile::{Profile, ProfileStore};
use crate::ssh::AuthMethod;

use super::common::{apply_ssh_import_overrides, merge_import_overwrite};
use super::types::{
    existing_id_for_endpoint, existing_id_for_name, ImportApplyResult, ImportCandidate,
    ImportCandidateStatus,
};

#[derive(Debug, Clone)]
pub struct ImportApplyItemInput {
    pub source_id: String,
    pub overwrite: bool,
    pub user: Option<String>,
    pub auth: Option<AuthMethod>,
    pub duplicate_of: Option<Uuid>,
    pub profile: Option<Profile>,
}

fn item_label(_item: &ImportApplyItemInput, profile: &Profile) -> String {
    profile.name.clone()
}

fn resolve_profile(
    item: &ImportApplyItemInput,
    by_id: &HashMap<String, ImportCandidate>,
) -> Option<Profile> {
    item.profile
        .clone()
        .or_else(|| by_id.get(&item.source_id).and_then(|c| c.profile.clone()))
}

pub fn resolve_overwrite_target(
    existing: &[Profile],
    duplicate_of: Option<Uuid>,
    profile: &Profile,
    fallback_name: Option<&str>,
) -> Option<Uuid> {
    duplicate_of
        .or_else(|| existing_id_for_endpoint(existing, profile))
        .or_else(|| existing_id_for_name(existing, profile.name.as_str()))
        .or_else(|| fallback_name.and_then(|name| existing_id_for_name(existing, name)))
}

pub async fn apply_import_items(
    store: &ProfileStore,
    existing: &[Profile],
    by_id: &HashMap<String, ImportCandidate>,
    items: Vec<ImportApplyItemInput>,
) -> ImportApplyResult {
    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut updated = 0usize;
    let mut errors = Vec::new();

    for item in items {
        let Some(mut profile) = resolve_profile(&item, by_id) else {
            skipped += 1;
            errors.push(format!("{}: missing profile payload", item.source_id));
            continue;
        };
        let label = item_label(&item, &profile);
        apply_ssh_import_overrides(&mut profile, item.user.as_deref(), item.auth.as_ref());

        if item.overwrite {
            let fallback_name = by_id.get(&item.source_id).map(|c| c.name.as_str());
            let target_id = resolve_overwrite_target(
                existing,
                item.duplicate_of
                    .or_else(|| by_id.get(&item.source_id).and_then(|c| c.duplicate_of)),
                &profile,
                fallback_name,
            );
            let Some(id) = target_id else {
                errors.push(format!(
                    "{label}: no existing profile matched for overwrite"
                ));
                skipped += 1;
                continue;
            };
            let existing_profile = store.get(id).await.ok().flatten();
            let mut merged = merge_import_overwrite(
                existing_profile,
                &profile,
                item.user.as_deref(),
                item.auth.as_ref(),
            );
            merged.id = id;
            if let Err(e) = store.upsert(merged).await {
                errors.push(format!("{label}: {e}"));
            } else {
                updated += 1;
            }
            continue;
        }

        let status = by_id
            .get(&item.source_id)
            .map(|c| c.status.clone())
            .unwrap_or(ImportCandidateStatus::Ready);
        match status {
            ImportCandidateStatus::Error | ImportCandidateStatus::Duplicate => {
                skipped += 1;
            }
            ImportCandidateStatus::Ready => {
                if let Err(e) = store.upsert(profile).await {
                    errors.push(format!("{label}: {e}"));
                } else {
                    created += 1;
                }
            }
        }
    }

    ImportApplyResult {
        created,
        skipped,
        updated,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileKind;
    use crate::ssh::{AuthMethod, SshProfile};

    fn tmp() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("aerotab-import-apply-{}", Uuid::new_v4()));
        p
    }

    fn ssh_profile(id: Uuid, name: &str, user: &str, auth: AuthMethod) -> Profile {
        Profile {
            schema_version: 1,
            id,
            name: name.into(),
            group: Some("Doocom".into()),
            tags: vec![],
            note: None,
            icon: None,
            favorite: false,
            spec: ProfileKind::Ssh {
                ssh: SshProfile {
                    host: "10.0.0.8".into(),
                    port: 22,
                    user: user.into(),
                    auth,
                    jump_via: vec![],
                },
            },
        }
    }

    #[tokio::test]
    async fn overwrite_updates_stored_profile_auth_without_source_lookup() {
        let dir = tmp();
        let store = ProfileStore::open(&dir).unwrap();
        let existing_id = Uuid::new_v4();
        store
            .upsert(ssh_profile(
                existing_id,
                "Doocom-Passbolt",
                "root",
                AuthMethod::PublicKey {
                    key_path: "D:\\old\\key".into(),
                    passphrase: None,
                },
            ))
            .await
            .unwrap();

        let existing = store.list().await.unwrap();
        let import = ssh_profile(
            Uuid::new_v4(),
            "Doocom-Passbolt",
            "devops",
            AuthMethod::VaultRef {
                entry_id: "doocom-devops-privkey".into(),
                passphrase_entry_id: None,
            },
        );
        let items = vec![ImportApplyItemInput {
            source_id: "missing-windterm-uuid".into(),
            overwrite: true,
            user: Some("devops".into()),
            auth: Some(AuthMethod::VaultRef {
                entry_id: "doocom-devops-privkey".into(),
                passphrase_entry_id: None,
            }),
            duplicate_of: Some(existing_id),
            profile: Some(import),
        }];

        let result = apply_import_items(&store, &existing, &HashMap::new(), items).await;
        assert_eq!(result.updated, 1, "{:?}", result.errors);
        assert!(result.errors.is_empty());

        let got = store.get(existing_id).await.unwrap().unwrap();
        if let ProfileKind::Ssh { ssh } = &got.spec {
            assert_eq!(ssh.user, "devops");
            assert!(matches!(ssh.auth, AuthMethod::VaultRef { .. }));
        } else {
            panic!("expected ssh");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
