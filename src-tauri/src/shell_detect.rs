//! Detection of built-in shells available on the current host.
//!
//! Mirrors Tabby's "Built-in shells" group in the profile picker: on
//! Linux/macOS we read `/etc/shells` and merge well-known paths (so `zsh`
//! appears even when omitted from `/etc/shells`), on Windows we probe CMD,
//! PowerShell, Git Bash / zsh, and WSL.
//!
//! Detection is best-effort and synchronous (filesystem stats only). Any
//! failure simply omits the entry — callers fall back to PtyChannel's
//! default shell.

use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

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
        detect_unix(true)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        detect_unix(false)
    }
}

#[cfg(unix)]
fn shell_display_label(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match name.as_str() {
        "zsh" => "Zsh".into(),
        "bash" => "Bash".into(),
        "sh" => "POSIX sh".into(),
        "fish" => "Fish".into(),
        "tcsh" => "tcsh".into(),
        "ksh" => "Ksh".into(),
        "dash" => "dash".into(),
        other => other.to_string(),
    }
}

#[cfg(unix)]
fn push_shell(
    out: &mut Vec<ShellEntry>,
    seen: &mut HashSet<String>,
    command: &str,
    icon: &str,
    label: Option<&str>,
) {
    if seen.contains(command) {
        return;
    }
    let path = Path::new(command);
    if !path.is_absolute() || !path.exists() {
        return;
    }
    seen.insert(command.to_string());
    let label = label
        .map(str::to_string)
        .unwrap_or_else(|| shell_display_label(path));
    out.push(ShellEntry {
        id: format!("shell:{command}"),
        label,
        command: command.to_string(),
        args: default_login_shell_args(command),
        icon: icon.to_string(),
    });
}

#[cfg(unix)]
fn default_login_shell_args(command: &str) -> Vec<String> {
    let base = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match base {
        "zsh" | "bash" => vec!["-l".into()],
        "fish" => vec!["--login".into()],
        _ => vec![],
    }
}

#[cfg(unix)]
fn detect_unix(macos: bool) -> Vec<ShellEntry> {
    let icon = if macos { "apple" } else { "linux" };
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    if let Ok(text) = std::fs::read_to_string("/etc/shells") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            push_shell(&mut out, &mut seen, line, icon, None);
        }
    }

    // Always probe common installs — `/etc/shells` may omit zsh (or only list bash).
    let extras: &[(&str, Option<&str>)] = if macos {
        &[
            ("/bin/zsh", Some("Zsh")),
            ("/bin/bash", Some("Bash")),
            ("/bin/sh", Some("POSIX sh")),
            ("/opt/homebrew/bin/zsh", Some("Zsh (Homebrew)")),
            ("/opt/homebrew/bin/bash", Some("Bash (Homebrew)")),
            ("/usr/local/bin/zsh", Some("Zsh")),
            ("/usr/local/bin/bash", Some("Bash")),
        ]
    } else {
        &[
            ("/bin/zsh", Some("Zsh")),
            ("/bin/bash", Some("Bash")),
            ("/usr/bin/zsh", Some("Zsh")),
            ("/usr/bin/bash", Some("Bash")),
            ("/bin/sh", Some("POSIX sh")),
            ("/usr/bin/fish", Some("Fish")),
        ]
    };
    for (path, label) in extras {
        push_shell(&mut out, &mut seen, path, icon, *label);
    }

    // Prefer zsh, then bash, then everything else (stable within each tier).
    out.sort_by(|a, b| {
        let rank = |cmd: &str| -> u8 {
            if cmd.contains("zsh") {
                0
            } else if cmd.contains("bash") {
                1
            } else if cmd.ends_with("/sh") {
                2
            } else {
                3
            }
        };
        rank(&a.command)
            .cmp(&rank(&b.command))
            .then_with(|| a.label.cmp(&b.label))
    });

    out
}

#[cfg(target_os = "windows")]
fn push_windows_shell(
    out: &mut Vec<ShellEntry>,
    seen: &mut HashSet<String>,
    id: &str,
    label: &str,
    command: &str,
    icon: &str,
    args: Vec<String>,
) {
    if seen.contains(command) {
        return;
    }
    if !Path::new(command).exists() {
        return;
    }
    seen.insert(command.to_string());
    out.push(ShellEntry {
        id: id.into(),
        label: label.into(),
        command: command.into(),
        args,
        icon: icon.into(),
    });
}

#[cfg(target_os = "windows")]
fn detect_windows() -> Vec<ShellEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
    let program_files_x86 =
        std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".into());

    let cmd = format!("{system_root}\\System32\\cmd.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:cmd",
        "CMD (stock)",
        &cmd,
        "windows",
        vec![],
    );
    let ps = format!("{system_root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:powershell",
        "PowerShell",
        &ps,
        "windows",
        vec![],
    );
    let pwsh = format!("{program_files}\\PowerShell\\7\\pwsh.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:pwsh7",
        "PowerShell 7",
        &pwsh,
        "windows",
        vec![],
    );
    let gitbash = format!("{program_files}\\Git\\bin\\bash.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:gitbash",
        "Git Bash",
        &gitbash,
        "git",
        vec![],
    );
    // Git for Windows often ships zsh under usr/bin (MSYS layout).
    for zsh in [
        format!("{program_files}\\Git\\usr\\bin\\zsh.exe"),
        format!("{program_files_x86}\\Git\\usr\\bin\\zsh.exe"),
        format!("{program_files}\\Git\\bin\\zsh.exe"),
    ] {
        push_windows_shell(
            &mut out,
            &mut seen,
            "shell:git-zsh",
            "Zsh (Git for Windows)",
            &zsh,
            "git",
            vec![],
        );
    }
    // MSYS2 / scoop-style installs (best-effort).
    if let Ok(home) = std::env::var("USERPROFILE") {
        let msys_zsh = format!("{home}\\scoop\\apps\\zsh\\current\\bin\\zsh.exe");
        push_windows_shell(
            &mut out,
            &mut seen,
            "shell:scoop-zsh",
            "Zsh (Scoop)",
            &msys_zsh,
            "linux",
            vec![],
        );
    }
    let msys2_zsh = format!("{program_files}\\msys64\\usr\\bin\\zsh.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:msys2-zsh",
        "Zsh (MSYS2)",
        &msys2_zsh,
        "linux",
        vec![],
    );

    let wsl = format!("{system_root}\\System32\\wsl.exe");
    push_windows_shell(
        &mut out,
        &mut seen,
        "shell:wsl-default",
        "WSL / Default distro",
        &wsl,
        "linux",
        vec![],
    );
    // Explicit WSL zsh entry when users run zsh inside their default distro.
    if Path::new(&wsl).exists() {
        push_windows_shell(
            &mut out,
            &mut seen,
            "shell:wsl-zsh",
            "WSL zsh",
            &wsl,
            "linux",
            vec!["-e".into(), "zsh".into(), "-l".into()],
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_well_formed_entries() {
        let list = detect();
        for e in &list {
            assert!(!e.command.is_empty());
            assert!(!e.id.is_empty());
        }
    }

    #[test]
    #[cfg(unix)]
    fn detect_includes_zsh_when_present() {
        if !Path::new("/bin/zsh").exists() {
            return;
        }
        let list = detect();
        assert!(
            list.iter().any(|e| e.command.contains("zsh")),
            "expected zsh in {:?}",
            list.iter().map(|e| &e.command).collect::<Vec<_>>()
        );
    }

    #[test]
    #[cfg(unix)]
    fn shell_display_label_maps_names() {
        assert_eq!(shell_display_label(Path::new("/bin/zsh")), "Zsh");
        assert_eq!(shell_display_label(Path::new("/bin/bash")), "Bash");
    }
}
