use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::profile::{Profile, ProfileKind};

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
