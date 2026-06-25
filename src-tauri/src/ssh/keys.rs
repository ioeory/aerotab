//! Default OpenSSH private-key discovery (import + connect fallbacks).

use std::path::PathBuf;

const DEFAULT_KEY_NAMES: &[&str] = &[
    "id_ed25519",
    "id_rsa",
    "id_ecdsa",
    "id_ed25519_sk",
    "id_rsa_sk",
];

pub fn ssh_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn default_ssh_key_candidates() -> Vec<PathBuf> {
    let Some(home) = ssh_home_dir() else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    DEFAULT_KEY_NAMES
        .iter()
        .map(|name| ssh_dir.join(name))
        .collect()
}

pub fn first_existing_ssh_key() -> Option<PathBuf> {
    default_ssh_key_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

/// Expand `~`, `%USERPROFILE%`, and relative home-relative WindTerm paths.
pub fn expand_identity_path(raw: &str) -> PathBuf {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return PathBuf::new();
    }

    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = ssh_home_dir() {
            let suffix = rest.trim_start_matches(['/', '\\']);
            s = if suffix.is_empty() {
                home.to_string_lossy().into_owned()
            } else {
                home.join(suffix).to_string_lossy().into_owned()
            };
        }
    }

    for (var, value) in std::env::vars() {
        let token = format!("%{var}%");
        if s.contains(&token) {
            s = s.replace(&token, &value);
        }
    }

    let path = PathBuf::from(&s);
    if path.is_file() {
        return path;
    }

    if path.is_absolute() {
        return path;
    }

    if let Some(home) = ssh_home_dir() {
        let trimmed = s.trim_start_matches(['/', '\\']);
        let under_home = home.join(trimmed);
        if under_home.is_file() {
            return under_home;
        }
        let under_ssh = home.join(".ssh").join(trimmed);
        if under_ssh.is_file() {
            return under_ssh;
        }
    }

    path
}

pub fn identity_path_if_file(raw: &str) -> Option<PathBuf> {
    let path = expand_identity_path(raw);
    if raw.trim().is_empty() {
        None
    } else if path.is_file() {
        Some(path)
    } else {
        // Keep configured path even when missing locally (health check / user fix).
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde_ssh_path() {
        let Some(home) = ssh_home_dir() else {
            return;
        };
        let expanded = expand_identity_path("~/.ssh/id_ed25519");
        assert_eq!(expanded, home.join(".ssh/id_ed25519"));
    }

    #[test]
    fn default_candidates_live_under_ssh_dir() {
        let Some(home) = ssh_home_dir() else {
            return;
        };
        let candidates = default_ssh_key_candidates();
        assert!(!candidates.is_empty());
        assert!(candidates[0].starts_with(home.join(".ssh")));
    }
}
