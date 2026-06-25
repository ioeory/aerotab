use std::collections::HashMap;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::profile::ProfileKind;
use crate::ssh::{AuthMethod, SshProfile};

use super::common::{build_profile, error_candidate, read_text_file, ready_candidate};
use super::types::{ImportCandidate, ImportDetectResult, ImportPreviewResult};

const SOURCE: &str = "securecrt";
const IMPORT_TAG: &str = "import:securecrt";

pub fn detect_securecrt_paths() -> ImportDetectResult {
    ImportDetectResult { paths: vec![] }
}

pub fn read_securecrt_file(path: &Path) -> Result<String, String> {
    read_text_file(path)
}

pub fn preview_securecrt(text: &str, path: Option<&str>) -> Result<ImportPreviewResult, String> {
    let sessions = parse_securecrt_xml(text)?;
    if sessions.is_empty() {
        return Err("no SecureCRT sessions found in XML export".into());
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
struct ScrtSession {
    name: String,
    folder: Vec<String>,
    fields: HashMap<String, String>,
}

fn parse_securecrt_xml(text: &str) -> Result<Vec<ScrtSession>, String> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut sessions = Vec::new();
    let mut key_stack: Vec<String> = Vec::new();
    let mut fields_stack: Vec<HashMap<String, String>> = Vec::new();
    let mut in_sessions = false;
    let mut current_field: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let tag = e.name().as_ref().to_ascii_lowercase();
                match tag.as_slice() {
                    b"key" => {
                        let name = attr_value(&e, b"name").unwrap_or_default();
                        if !in_sessions && name.eq_ignore_ascii_case("Sessions") {
                            in_sessions = true;
                        }
                        if in_sessions {
                            key_stack.push(name);
                            fields_stack.push(HashMap::new());
                        }
                    }
                    b"string" | b"dword" if in_sessions => {
                        current_field = attr_value(&e, b"name");
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) if in_sessions => {
                if let Some(field) = current_field.as_ref() {
                    let value = e.unescape().unwrap_or_default().into_owned();
                    if let Some(map) = fields_stack.last_mut() {
                        map.insert(field.clone(), value);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let tag = e.name().as_ref().to_ascii_lowercase();
                match tag.as_slice() {
                    b"string" | b"dword" => current_field = None,
                    b"key" if in_sessions => {
                        let fields = fields_stack.pop().unwrap_or_default();
                        let name = key_stack.pop().unwrap_or_default();
                        if is_session_node(&fields) && !is_template_name(&name) {
                            let folder = key_stack.iter().skip(1).cloned().collect::<Vec<_>>();
                            sessions.push(ScrtSession {
                                name,
                                folder,
                                fields,
                            });
                        }
                        if key_stack.is_empty() {
                            in_sessions = false;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("invalid SecureCRT XML: {e}")),
            _ => {}
        }
    }
    Ok(sessions)
}

fn attr_value(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| String::from_utf8(a.value.into_owned()).ok())
}

fn is_session_node(fields: &HashMap<String, String>) -> bool {
    fields.get("Hostname").is_some_and(|h| !h.trim().is_empty())
}

fn is_template_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "default" | "default_rdp" | "default_telnet"
    )
}

fn map_session(session: ScrtSession, idx: usize) -> ImportCandidate {
    let source_id = format!("securecrt:{}:{idx}", session.name);
    let tags = vec![IMPORT_TAG.to_string()];
    let protocol = session
        .fields
        .get("Protocol Name")
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_else(|| "SSH2".into());

    if !matches!(protocol.as_str(), "SSH2" | "SSH1" | "SSH") {
        return error_candidate(
            SOURCE,
            source_id,
            session.name,
            format!("unsupported SecureCRT protocol: {protocol}"),
            tags,
        );
    }

    let host = session.fields.get("Hostname").cloned().unwrap_or_default();
    if host.is_empty() {
        return error_candidate(
            SOURCE,
            source_id,
            session.name,
            "missing Hostname".into(),
            tags,
        );
    }

    let port = session
        .fields
        .get("[SSH2] Port")
        .or_else(|| session.fields.get("Port"))
        .and_then(|p| parse_scrt_port(p.as_str()))
        .unwrap_or(22);
    let user = session
        .fields
        .get("Username")
        .cloned()
        .unwrap_or_else(|| "root".into());

    let mut warnings = vec!["password not imported; using ssh-agent".into()];
    if !session.fields.contains_key("Username") {
        warnings.push("no Username; defaulting to root".into());
    }

    let group = if session.folder.is_empty() {
        None
    } else {
        Some(session.folder.join("/"))
    };

    let ssh = SshProfile {
        host,
        port,
        user,
        auth: AuthMethod::Agent,
        jump_via: vec![],
    };
    let profile = build_profile(
        session.name.clone(),
        group.clone(),
        tags.clone(),
        Some("Imported from SecureCRT".into()),
        ProfileKind::Ssh { ssh },
    );
    ready_candidate(
        SOURCE,
        source_id,
        session.name,
        group,
        tags,
        Some("Imported from SecureCRT".into()),
        "ssh",
        profile,
        warnings,
    )
}

fn parse_scrt_port(raw: &str) -> Option<u16> {
    let s = raw.trim();
    let hex = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("dword:"))
        .unwrap_or(s);
    if let Ok(n) = u32::from_str_radix(hex.trim_start_matches('0'), 16) {
        return u16::try_from(n).ok();
    }
    s.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<VanDyke>
  <key name="Sessions">
    <key name="Production">
      <key name="web-01">
        <string name="Hostname">10.0.0.5</string>
        <string name="Username">alice</string>
        <dword name="[SSH2] Port">0x000008ae</dword>
        <string name="Protocol Name">SSH2</string>
      </key>
    </key>
    <key name="Default">
      <string name="Protocol Name">SSH2</string>
    </key>
    <key name="desk">
      <string name="Hostname">192.168.1.20</string>
      <string name="Protocol Name">RDP</string>
    </key>
  </key>
</VanDyke>
"#;

    #[test]
    fn parses_ssh_and_marks_unsupported() {
        let preview = preview_securecrt(SAMPLE, None).expect("preview");
        assert_eq!(preview.candidates.len(), 2);
        let ssh = preview
            .candidates
            .iter()
            .find(|c| c.name == "web-01")
            .expect("web-01");
        assert_eq!(ssh.kind, "ssh");
        assert_eq!(ssh.group.as_deref(), Some("Production"));
        let ProfileKind::Ssh { ssh: spec } = &ssh.profile.as_ref().unwrap().spec else {
            panic!("ssh");
        };
        assert_eq!(spec.port, 2222);
        assert_eq!(
            preview
                .candidates
                .iter()
                .find(|c| c.name == "desk")
                .unwrap()
                .status,
            super::super::types::ImportCandidateStatus::Error
        );
    }
}
