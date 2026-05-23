//! Launch external RDP/VNC clients (optionally after SSH local forward).

use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("unsupported remote kind: {0}")]
    UnsupportedKind(String),
    #[error("launch failed: {0}")]
    Launch(String),
}

/// Spawns the platform viewer for `kind` (`rdp` or `vnc`) targeting `address` (`host:port`).
pub fn launch_viewer(kind: &str, address: &str) -> Result<(), RemoteError> {
    let kind = kind.to_ascii_lowercase();
    match kind.as_str() {
        "rdp" => launch_rdp(address),
        "vnc" => launch_vnc(address),
        other => Err(RemoteError::UnsupportedKind(other.to_string())),
    }
}

fn launch_rdp(address: &str) -> Result<(), RemoteError> {
    #[cfg(target_os = "windows")]
    {
        Command::new("mstsc")
            .arg(format!("/v:{address}"))
            .spawn()
            .map_err(|e| RemoteError::Launch(e.to_string()))?;
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        {
            let status = Command::new("open")
                .args(["-a", "Microsoft Remote Desktop"])
                .status()
                .map_err(|e| RemoteError::Launch(e.to_string()))?;
            if status.success() {
                return Ok(());
            }
        }
        if Command::new("xfreerdp")
            .args([format!("/v:{address}"), "/cert:ignore".into()])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
        Command::new("remmina")
            .args(["-c", &format!("rdp://{address}")])
            .spawn()
            .map_err(|e| RemoteError::Launch(format!("xfreerdp/remmina: {e}")))?;
        Ok(())
    }
}

fn launch_vnc(address: &str) -> Result<(), RemoteError> {
    let host_port = address.replace(':', "_");
    #[cfg(target_os = "macos")]
    {
        if Command::new("open")
            .args(["-a", "Screen Sharing", &format!("vnc://{address}")])
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    if Command::new("vncviewer")
        .arg(address)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    Command::new("xtightvncviewer")
        .arg(address)
        .spawn()
        .map_err(|e| RemoteError::Launch(format!("vncviewer: {e}; tried host {host_port}")))?;
    Ok(())
}
