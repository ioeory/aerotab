//! Local SSH agent selection shared by authentication and forwarding.

use serde::{Deserialize, Serialize};
use std::io;

use super::LocalAgentStream;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentPreference {
    #[default]
    Auto,
    System,
    Pageant,
    None,
}

// PuTTY's AGENT_MAX_MSGLEN includes the four-byte SSH agent frame header.
#[cfg_attr(not(windows), allow(dead_code))]
const PAGEANT_MAPPING_SIZE: usize = 256 * 1024;

#[cfg_attr(not(windows), allow(dead_code))]
fn pageant_packet_len(header: &[u8; 4]) -> io::Result<usize> {
    let body = u32::from_be_bytes(*header) as usize;
    if body == 0 || body > PAGEANT_MAPPING_SIZE - 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Pageant packet length",
        ));
    }
    Ok(body + 4)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn system_unavailable(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused | io::ErrorKind::NotConnected)
        // ERROR_PIPE_BUSY: the agent exists but cannot accept a connection.
        || cfg!(windows) && error.raw_os_error() == Some(231)
}

pub(super) async fn connect(preference: AgentPreference) -> io::Result<Box<dyn LocalAgentStream>> {
    match preference {
        AgentPreference::None => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH agent is disabled",
        )),
        AgentPreference::Pageant => connect_pageant().await,
        AgentPreference::System => connect_system().await,
        AgentPreference::Auto => {
            let system = connect_system().await;
            #[cfg(windows)]
            if system.as_ref().is_err_and(system_unavailable) {
                return connect_pageant().await;
            }
            system
        }
    }
}

async fn connect_system() -> io::Result<Box<dyn LocalAgentStream>> {
    #[cfg(unix)]
    {
        let path = std::env::var_os("SSH_AUTH_SOCK")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SSH_AUTH_SOCK is not set"))?;
        Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
    }
    #[cfg(windows)]
    {
        let pipe = std::env::var_os("SSH_AUTH_SOCK")
            .unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".into());
        Ok(Box::new(
            tokio::net::windows::named_pipe::ClientOptions::new().open(pipe)?,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "system SSH agent is unsupported",
        ))
    }
}

async fn connect_pageant() -> io::Result<Box<dyn LocalAgentStream>> {
    #[cfg(windows)]
    {
        pageant::connect().await
    }
    #[cfg(not(windows))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Pageant is supported only on Windows",
        ))
    }
}

#[cfg(windows)]
#[path = "pageant.rs"]
mod pageant;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_parse_strictly_and_default_to_auto() {
        assert_eq!(AgentPreference::default(), AgentPreference::Auto);
        for (value, expected) in [
            ("auto", AgentPreference::Auto),
            ("system", AgentPreference::System),
            ("pageant", AgentPreference::Pageant),
            ("none", AgentPreference::None),
        ] {
            assert_eq!(
                serde_json::from_str::<AgentPreference>(&format!("\"{value}\"")).unwrap(),
                expected
            );
        }
        assert!(serde_json::from_str::<AgentPreference>("\"other\"").is_err());
    }

    #[tokio::test]
    async fn disabled_agent_never_opens_a_transport() {
        let err = connect(AgentPreference::None).await.err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn explicit_pageant_is_unsupported_outside_windows() {
        let err = connect(AgentPreference::Pageant).await.err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }

    #[test]
    fn pageant_frames_are_nonempty_and_bounded() {
        assert!(pageant_packet_len(&[0, 0, 0, 0]).is_err());
        assert!(pageant_packet_len(&[0, 4, 0, 0]).is_err());
        assert!(pageant_packet_len(&[0xff; 4]).is_err());
        assert_eq!(pageant_packet_len(&[0, 0, 0, 1]).unwrap(), 5);
        assert_eq!(
            pageant_packet_len(&[0, 3, 0xff, 0xfc]).unwrap(),
            PAGEANT_MAPPING_SIZE
        );
    }

    #[test]
    fn auto_falls_back_only_for_unavailable_transport() {
        use std::io::ErrorKind;
        assert!(system_unavailable(&std::io::Error::from(
            ErrorKind::NotFound
        )));
        assert!(system_unavailable(&std::io::Error::from(
            ErrorKind::ConnectionRefused
        )));
        assert!(!system_unavailable(&std::io::Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!system_unavailable(&std::io::Error::from(
            ErrorKind::InvalidData
        )));
    }
}
