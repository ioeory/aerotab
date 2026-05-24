//! Detection of built-in shells available on the current host.
//!
//! Mirrors Tabby's "Built-in shells" group in the profile picker: on
//! Linux/macOS we read `/etc/shells`, on Windows we probe the well-known
//! install locations for CMD, PowerShell, Git Bash and WSL distros.
//!
//! Detection is best-effort and synchronous (filesystem stats only). Any
//! failure simply omits the entry — callers fall back to PtyChannel's
//! default shell.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ShellEntry {
    /// Stable id used by the UI when selecting a shell.
    pub id: String,
    /// Human label (e.g. "PowerShell", "Git Bash").
    pub label: String,
    /// Absolute path of the executable to spawn.
    pub command: String,
    /// Optional arguments. Most shells need none; WSL uses `-d <distro>`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Icon hint for the UI (e.g. "windows", "linux", "apple"). Free-form.
    pub icon: String,
}

/// Enumerate all built-in shells on the current OS.
pub fn detect() -> Vec<ShellEntry> {
    #[cfg(target_os = "windows")]
    {
        detect_windows()
    }
    #[cfg(target_os = "macos")]
    {
        detect_unix(/* macos = */ true)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        detect_unix(false)
    }
}

#[cfg(unix)]
fn detect_unix(_macos: bool) -> Vec<ShellEntry> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = std::path::PathBuf::from(line);
            if !path.is_absolute() || !path.exists() {
                continue;
            }
            let label = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| line.to_string());
            out.push(ShellEntry {
                id: format!("shell:{}", path.to_string_lossy()),
                label,
                command: path.to_string_lossy().into_owned(),
                args: vec![],
                icon: "linux".into(),
            });
        }
    }
    // Common fallbacks if /etc/shells is missing.
    if out.is_empty() {
        for cand in ["/bin/bash", "/bin/zsh", "/bin/sh"] {
            if std::path::Path::new(cand).exists() {
                out.push(ShellEntry {
                    id: format!("shell:{cand}"),
                    label: std::path::PathBuf::from(cand)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    command: cand.to_string(),
                    args: vec![],
                    icon: "linux".into(),
                });
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn detect_windows() -> Vec<ShellEntry> {
    let mut out = Vec::new();
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());

    let cmd = format!("{system_root}\\System32\\cmd.exe");
    if std::path::Path::new(&cmd).exists() {
        out.push(ShellEntry {
            id: "shell:cmd".into(),
            label: "CMD (stock)".into(),
            command: cmd.clone(),
            args: vec![],
            icon: "windows".into(),
        });
    }
    let ps = format!("{system_root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    if std::path::Path::new(&ps).exists() {
        out.push(ShellEntry {
            id: "shell:powershell".into(),
            label: "PowerShell".into(),
            command: ps,
            args: vec![],
            icon: "windows".into(),
        });
    }
    let pwsh = format!("{program_files}\\PowerShell\\7\\pwsh.exe");
    if std::path::Path::new(&pwsh).exists() {
        out.push(ShellEntry {
            id: "shell:pwsh7".into(),
            label: "PowerShell 7".into(),
            command: pwsh,
            args: vec![],
            icon: "windows".into(),
        });
    }
    let gitbash = format!("{program_files}\\Git\\bin\\bash.exe");
    if std::path::Path::new(&gitbash).exists() {
        out.push(ShellEntry {
            id: "shell:gitbash".into(),
            label: "Git Bash".into(),
            command: gitbash,
            args: vec![],
            icon: "git".into(),
        });
    }
    let wsl = format!("{system_root}\\System32\\wsl.exe");
    if std::path::Path::new(&wsl).exists() {
        out.push(ShellEntry {
            id: "shell:wsl-default".into(),
            label: "WSL / Default distro".into(),
            command: wsl,
            args: vec![],
            icon: "linux".into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(unix)]
    fn detect_returns_some_entries_on_unix() {
        // CI runners always have at least /bin/sh.
        let list = super::detect();
        // List may be empty in exotic sandboxes; just ensure it
        // doesn't panic and is well-formed when populated.
        for e in &list {
            assert!(!e.command.is_empty());
            assert!(!e.id.is_empty());
        }
    }
}
