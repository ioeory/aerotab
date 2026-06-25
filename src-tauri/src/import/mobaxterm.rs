use std::path::{Path, PathBuf};

use crate::profile::ProfileKind;
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{build_profile, error_candidate, home_dir, read_text_file, ready_candidate};
use super::types::{ImportCandidate, ImportDetectPath, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "mobaxterm";
const IMPORT_TAG: &str = "import:mobaxterm";

pub fn detect_mobaxterm_paths() -> ImportDetectResult {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        for rel in [
            "Documents/MobaXterm/MobaXterm.ini",
            "Desktop/MobaXterm/MobaXterm.ini",
            "MobaXterm/MobaXterm.ini",
        ] {
            let p = home.join(rel);
            push_ini_path(&mut paths, &p);
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata)
            .join("MobaXterm")
            .join("MobaXterm.ini");
        push_ini_path(&mut paths, &p);
    }
    ImportDetectResult { paths }
}

fn push_ini_path(out: &mut Vec<ImportDetectPath>, path: &Path) {
    if !path.is_file() {
        return;
    }
    out.push(ImportDetectPath {
        path: path.display().to_string(),
        label: "MobaXterm.ini".into(),
    });
}

pub fn read_mobaxterm_file(path: &Path) -> Result<String, String> {
    read_text_file(path)
}

pub fn preview_mobaxterm(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let blobs = extract_session_blobs(text);
    if blobs.is_empty() {
        return Err("no MobaXterm SSH sessions found in ini".into());
    }
    let candidates: Vec<ImportCandidate> = blobs
        .into_iter()
        .enumerate()
        .flat_map(|(blob_idx, blob)| {
            blob.split("#109#")
                .filter(|chunk| !chunk.is_empty())
                .enumerate()
                .filter_map(move |(idx, chunk)| map_chunk(chunk, blob_idx, idx))
                .collect::<Vec<_>>()
        })
        .collect();
    if candidates.is_empty() {
        return Err("no importable MobaXterm SSH sessions found".into());
    }
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

fn extract_session_blobs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((_, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.contains("#109#") {
            out.push(value.to_string());
        }
    }
    out
}

fn map_chunk(chunk: &str, blob_idx: usize, idx: usize) -> Option<ImportCandidate> {
    if !chunk.contains("<SSH session>") {
        if chunk.contains("<RDP session>")
            || chunk.contains("<Telnet session>")
            || chunk.contains("<VNC session>")
        {
            let name = chunk_field(chunk, 1).unwrap_or_else(|| format!("Session {idx}"));
            return Some(error_candidate(
                SOURCE,
                format!("mobaxterm:{blob_idx}:{idx}"),
                name,
                "non-SSH MobaXterm session".into(),
                vec![IMPORT_TAG.to_string()],
            ));
        }
        return None;
    }

    let name = chunk_field(chunk, 1).filter(|s| !s.is_empty())?;
    let host = chunk_field(chunk, 2).filter(|s| !s.is_empty())?;
    let port = chunk_field(chunk, 3)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(22);
    let user = chunk_field(chunk, 5)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "root".into());

    let source_id = format!("mobaxterm:{blob_idx}:{idx}:{host}:{port}");
    let tags = vec![IMPORT_TAG.to_string()];
    let mut warnings = vec!["password not imported; using ssh-agent".into()];
    if chunk_field(chunk, 5).is_none() {
        warnings.push("no username in session blob; defaulting to root".into());
    }

    let ssh = SshProfile {
        host: host.clone(),
        port,
        user,
        auth: AuthMethod::Agent,
        jump_via: vec![],
    };
    let profile = build_profile(
        name.clone(),
        None,
        tags.clone(),
        Some("Imported from MobaXterm".into()),
        ProfileKind::Ssh { ssh },
    );
    Some(ready_candidate(
        SOURCE,
        source_id,
        name,
        None,
        tags,
        Some("Imported from MobaXterm".into()),
        "ssh",
        profile,
        warnings,
    ))
}

fn chunk_field(chunk: &str, field_idx: usize) -> Option<String> {
    chunk
        .split("#%")
        .map(|s| s.trim().trim_start_matches('%'))
        .filter(|s| !s.is_empty())
        .nth(field_idx - 1)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[Bookmarks]
subrep=My sessions
My sessions=#109#%web-01#%10.0.0.5#%2222#%0#%alice#%%-1#-1#0#<SSH session>#-1#0#0#22#4626#0#109#%desk#%192.168.1.20#%3389#%0#%admin#%%-1#-1#0#<RDP session>#-1#0#0#22#4626#0
"#;

    #[test]
    fn parses_ssh_and_marks_rdp_error() {
        let preview = preview_mobaxterm(SAMPLE, None).expect("preview");
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].name, "web-01");
        assert_eq!(preview.candidates[0].kind, "ssh");
        assert_eq!(
            preview.candidates[1].status,
            super::super::types::ImportCandidateStatus::Error
        );
    }
}
