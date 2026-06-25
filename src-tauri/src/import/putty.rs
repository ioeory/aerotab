use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::profile::ProfileKind;
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{build_profile, error_candidate, read_text_file, ready_candidate};
use super::types::{ImportCandidate, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "putty";
const IMPORT_TAG: &str = "import:putty";

pub fn detect_putty_paths() -> ImportDetectResult {
    ImportDetectResult { paths: vec![] }
}

pub fn read_putty_file(path: &Path) -> Result<String, String> {
    read_text_file(path)
}

pub fn preview_putty(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let sessions = parse_putty_reg(text);
    if sessions.is_empty() {
        return Err("no PuTTY sessions found in registry export".into());
    }
    let candidates = sessions
        .into_iter()
        .enumerate()
        .map(|(idx, session)| map_session(session, idx))
        .collect::<Vec<_>>();
    Ok(ImportPreviewResult {
        source: SOURCE.into(),
        path: path.map(String::from),
        stats: super::types::preview_stats(&candidates),
        candidates,
    })
}

#[derive(Debug, Clone)]
struct PuttySession {
    name: String,
    values: HashMap<String, String>,
}

fn parse_putty_reg(text: &str) -> Vec<PuttySession> {
    let mut out = Vec::new();
    let mut current: Option<PuttySession> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(session) = current.take() {
                if !session.values.is_empty() {
                    out.push(session);
                }
            }
            let header = &line[1..line.len() - 1];
            if let Some(name) = session_name_from_key(header) {
                current = Some(PuttySession {
                    name,
                    values: HashMap::new(),
                });
            }
            continue;
        }
        if let Some(session) = current.as_mut() {
            if let Some((key, value)) = parse_reg_value(line) {
                session.values.insert(key.to_ascii_lowercase(), value);
            }
        }
    }
    if let Some(session) = current.take() {
        if !session.values.is_empty() {
            out.push(session);
        }
    }
    out
}

fn session_name_from_key(header: &str) -> Option<String> {
    let marker = "\\Sessions\\";
    let idx = header.rfind(marker)?;
    let encoded = header[idx + marker.len()..].trim();
    if encoded.is_empty() || encoded.eq_ignore_ascii_case("Default%20Settings") {
        return None;
    }
    Some(decode_reg_name(encoded))
}

fn decode_reg_name(encoded: &str) -> String {
    let mut out = String::new();
    let bytes = encoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_reg_value(line: &str) -> Option<(&str, String)> {
    let line = line.trim();
    if !line.starts_with('"') {
        return None;
    }
    let rest = &line[1..];
    let key_end = rest.find('"')?;
    let key = &rest[..key_end];
    let after_key = rest[key_end + 1..].trim();
    if !after_key.starts_with('=') {
        return None;
    }
    let value_part = after_key[1..].trim();
    let value = if let Some(quoted) = value_part.strip_prefix('"') {
        let end = quoted.find('"')?;
        quoted[..end].to_string()
    } else {
        value_part.split_whitespace().next()?.to_string()
    };
    Some((key, value))
}

fn map_session(session: PuttySession, idx: usize) -> ImportCandidate {
    let source_id = format!("putty:{}:{}", session.name, idx);
    let tags = vec![IMPORT_TAG.to_string()];
    let protocol = session
        .values
        .get("protocol")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| "ssh".into());

    if !matches!(protocol.as_str(), "ssh" | "") {
        return error_candidate(
            SOURCE,
            source_id,
            session.name.clone(),
            format!("unsupported PuTTY protocol: {protocol}"),
            tags,
        );
    }

    let host = session.values.get("hostname").cloned().unwrap_or_default();
    if host.is_empty() {
        return error_candidate(
            SOURCE,
            source_id,
            session.name,
            "missing HostName".into(),
            tags,
        );
    }

    let port = session
        .values
        .get("portnumber")
        .and_then(|p| parse_putty_port(p.as_str()))
        .unwrap_or(22);
    let user = session
        .values
        .get("username")
        .cloned()
        .unwrap_or_else(|| "root".into());

    let mut warnings = Vec::new();
    if !session.values.contains_key("username") {
        warnings.push("no UserName; defaulting to root".into());
    }
    warnings.push("password not imported; using ssh-agent".into());

    let auth = if let Some(key) = session.values.get("publickeyfile") {
        if key.to_ascii_lowercase().ends_with(".ppk") {
            warnings.push("PPK key path preserved; convert to OpenSSH format if needed".into());
        }
        AuthMethod::PublicKey {
            key_path: PathBuf::from(key),
            passphrase: None,
        }
    } else {
        AuthMethod::Agent
    };

    let (name, group) = split_putty_session_name(&session.name);
    let ssh = SshProfile {
        host,
        port,
        user,
        auth,
        jump_via: vec![],
    };
    let profile = build_profile(
        name.clone(),
        group.clone(),
        tags.clone(),
        Some("Imported from PuTTY".into()),
        ProfileKind::Ssh { ssh },
    );
    ready_candidate(
        SOURCE,
        source_id,
        name,
        group,
        tags,
        Some("Imported from PuTTY".into()),
        "ssh",
        profile,
        warnings,
    )
}

fn split_putty_session_name(raw: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() <= 1 {
        return (raw.to_string(), None);
    }
    let name = parts.last().unwrap().to_string();
    let group = parts[..parts.len() - 1].join("/");
    (name, Some(group))
}

fn parse_putty_port(raw: &str) -> Option<u16> {
    let s = raw.trim();
    let hex = s
        .strip_prefix("dword:")
        .or_else(|| s.strip_prefix("0x"))
        .unwrap_or(s);
    if let Ok(n) = u32::from_str_radix(hex.trim_start_matches('0'), 16) {
        return u16::try_from(n).ok();
    }
    s.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Prod-web%2D01]
"HostName"="10.0.0.5"
"PortNumber"=dword:000008ae
"Protocol"="ssh"
"UserName"="alice"
"PublicKeyFile"="C:\\Users\\alice\\.ssh\\id_rsa.ppk"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\telnet%2Dhost]
"HostName"="10.0.0.9"
"PortNumber"=dword:00000017
"Protocol"="telnet"
"#;

    #[test]
    fn parses_ssh_and_skips_telnet() {
        let preview = preview_putty(SAMPLE, None).expect("preview");
        assert_eq!(preview.candidates.len(), 2);
        let ssh = &preview.candidates[0];
        assert_eq!(ssh.kind, "ssh");
        assert_eq!(ssh.name, "01");
        assert_eq!(ssh.group.as_deref(), Some("Prod/web"));
        let ProfileKind::Ssh { ssh: spec } = &ssh.profile.as_ref().unwrap().spec else {
            panic!("ssh");
        };
        assert_eq!(spec.port, 2222);
        assert_eq!(
            preview.candidates[1].status,
            super::super::types::ImportCandidateStatus::Error
        );
    }
}
