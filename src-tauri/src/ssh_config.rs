//! Parser for the user's `~/.ssh/config` file.
//!
//! Tabby surfaces Host entries from the SSH client config as read-only
//! connection profiles in the picker. We only need the basics: Host,
//! HostName, User, Port, IdentityFile. Wildcard hosts (`Host *`) are
//! skipped because they are templates, not connectable targets.
//!
//! The format reference is `ssh_config(5)`. Tokens are case-insensitive;
//! values may be quoted. Comments begin with `#`.

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SshConfigEntry {
    /// Host alias as written in `~/.ssh/config`.
    pub alias: String,
    /// Resolved hostname (HostName), or the alias if not specified.
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    /// Host aliases from `ProxyJump` / `Proxyjump` (left-to-right bastion order).
    pub proxy_jump: Vec<String>,
}

/// Default location, honouring `$HOME`. Returns None on platforms where
/// the home directory cannot be resolved.
pub fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh").join("config"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Parse the given config text. Unknown keywords are silently ignored.
pub fn parse(text: &str) -> Vec<SshConfigEntry> {
    let mut out: Vec<SshConfigEntry> = Vec::new();
    let mut current: Option<SshConfigEntry> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split into keyword + value. SSH accepts `=` and whitespace.
        let (kw, val) = match split_kv(line) {
            Some(kv) => kv,
            None => continue,
        };
        let kw_lc = kw.to_ascii_lowercase();
        if kw_lc == "host" {
            // Flush previous, start new entries for each pattern.
            if let Some(e) = current.take() {
                if !is_wildcard(&e.alias) {
                    out.push(e);
                }
            }
            // ssh_config allows multiple patterns per Host line; we only
            // care about the first concrete one (the rest are usually
            // wildcards or aliases for the same target).
            let first = val.split_whitespace().next().unwrap_or("").to_string();
            if first.is_empty() {
                continue;
            }
            current = Some(SshConfigEntry {
                alias: first.clone(),
                host: first,
                port: 22,
                user: None,
                identity_file: None,
                proxy_jump: Vec::new(),
            });
        } else if let Some(e) = current.as_mut() {
            match kw_lc.as_str() {
                "hostname" => e.host = val.to_string(),
                "user" => e.user = Some(val.to_string()),
                "port" => {
                    if let Ok(p) = val.parse::<u16>() {
                        e.port = p;
                    }
                }
                "identityfile" => e.identity_file = Some(expand_tilde(val)),
                "proxyjump" => {
                    e.proxy_jump = val
                        .split(',')
                        .flat_map(|part| part.split_whitespace())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                _ => {}
            }
        }
    }
    if let Some(e) = current.take() {
        if !is_wildcard(&e.alias) {
            out.push(e);
        }
    }
    out
}

/// Load and parse the user's `~/.ssh/config`. Returns an empty list if the
/// file does not exist or cannot be read — the picker still works.
pub fn load_default() -> Vec<SshConfigEntry> {
    let Some(path) = default_config_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse(&text)
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    // Try `keyword=value` first, then any whitespace-separated form.
    if let Some(eq) = line.find('=') {
        let (k, v) = line.split_at(eq);
        let v = v[1..].trim().trim_matches('"');
        let k = k.trim();
        if !k.is_empty() {
            return Some((k, v));
        }
    }
    let mut it = line.splitn(2, char::is_whitespace);
    let k = it.next()?.trim();
    let v = it.next().unwrap_or("").trim().trim_matches('"');
    if k.is_empty() {
        None
    } else {
        Some((k, v))
    }
}

fn is_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(h) = home_dir() {
            return h.join(rest).to_string_lossy().to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_entries() {
        let cfg = "
            # global comment
            Host my-server
              HostName 1.2.3.4
              User alice
              Port 2222
              IdentityFile ~/.ssh/id_ed25519

            Host gateway
              HostName gw.example.com
        ";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 2);
        let me = &entries[0];
        assert_eq!(me.alias, "my-server");
        assert_eq!(me.host, "1.2.3.4");
        assert_eq!(me.user.as_deref(), Some("alice"));
        assert_eq!(me.port, 2222);
        assert!(me.identity_file.is_some());
        assert_eq!(entries[1].alias, "gateway");
        assert_eq!(entries[1].port, 22);
    }

    #[test]
    fn skips_wildcard_hosts() {
        let cfg = "Host *\n  User root\nHost foo\n  HostName 10.0.0.1\n";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "foo");
    }

    #[test]
    fn accepts_equals_form() {
        let cfg = "Host=bar\nHostName=2.2.2.2\nPort=2200\n";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, "2.2.2.2");
        assert_eq!(entries[0].port, 2200);
    }

    #[test]
    fn parses_proxy_jump() {
        let cfg = "
            Host bastion
              HostName 10.0.0.1
              User jump

            Host target
              HostName 10.0.0.2
              ProxyJump bastion
        ";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].proxy_jump, vec!["bastion".to_string()]);
    }
}
