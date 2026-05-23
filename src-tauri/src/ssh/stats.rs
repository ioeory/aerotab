//! Lightweight remote host statistics for SSH sessions.
//!
//! The probe opens a short-lived exec channel instead of writing commands into
//! the user's interactive terminal, so sampling never pollutes scrollback.

use std::time::Duration;

use russh::{ChannelMsg, Disconnect};
use serde::Serialize;
use tokio::time::timeout;

use super::known_hosts::KnownHosts;
use super::{connect_authenticated, SshError, SshProfile};

const HOST_STATS_COMMAND: &str = r#"LC_ALL=C; export LC_ALL;
host=$(hostname 2>/dev/null || uname -n 2>/dev/null || echo ""); [ -n "$host" ] && printf 'hostname=%s\n' "$host";
kernel=$(uname -srmo 2>/dev/null || uname -a 2>/dev/null || echo ""); [ -n "$kernel" ] && printf 'kernel=%s\n' "$kernel";
awk '{ printf "uptime_seconds=%d\n", $1 }' /proc/uptime 2>/dev/null;
awk '{ printf "load1=%.2f\n", $1 }' /proc/loadavg 2>/dev/null;
read_cpu() { awk '/^cpu / { idle=$5+$6; total=0; for (i=2; i<=NF; i++) total += $i; printf "%s %s\n", idle, total; exit }' /proc/stat 2>/dev/null; }
set -- $(read_cpu); idle1=$1; total1=$2; sleep 0.2; set -- $(read_cpu); idle2=$1; total2=$2;
if [ -n "$total1" ] && [ -n "$total2" ]; then awk -v i1="$idle1" -v t1="$total1" -v i2="$idle2" -v t2="$total2" 'BEGIN { dt=t2-t1; di=i2-i1; if (dt > 0) printf "cpu_percent=%.1f\n", (1 - di / dt) * 100 }'; fi;
awk '/^MemTotal:/ { t=$2 } /^MemAvailable:/ { a=$2 } END { if (t > 0) { used=t-a; printf "mem_total_kb=%d\nmem_used_kb=%d\nmem_percent=%.1f\n", t, used, used * 100 / t } }' /proc/meminfo 2>/dev/null;
df -Pk / 2>/dev/null | awk 'NR==2 { pct=$5; sub(/%$/, "", pct); printf "disk_total_kb=%s\ndisk_used_kb=%s\ndisk_percent=%s\n", $2, $3, pct }'
"#;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct HostStats {
    pub hostname: Option<String>,
    pub kernel: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub load1: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub mem_total_kb: Option<u64>,
    pub mem_used_kb: Option<u64>,
    pub mem_percent: Option<f64>,
    pub disk_total_kb: Option<u64>,
    pub disk_used_kb: Option<u64>,
    pub disk_percent: Option<f64>,
}

pub async fn probe_host_stats(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
) -> Result<HostStats, SshError> {
    let handle = connect_authenticated(profile, known_hosts).await?;
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Channel(format!("host stats channel: {e}")))?;
    channel
        .exec(true, HOST_STATS_COMMAND)
        .await
        .map_err(|e| SshError::Channel(format!("host stats exec: {e}")))?;

    let collect = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
                ChannelMsg::ExtendedData { ref data, ext: _ } => stderr.extend_from_slice(data),
                ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok::<_, SshError>((stdout, stderr, exit_status))
    };
    let (stdout, stderr, exit_status) = timeout(Duration::from_secs(10), collect)
        .await
        .map_err(|_| SshError::Channel("host stats timed out".into()))??;
    let _ = handle
        .disconnect(Disconnect::ByApplication, "host stats complete", "en")
        .await;

    if matches!(exit_status, Some(code) if code != 0) {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(SshError::Channel(if detail.is_empty() {
            format!(
                "host stats exited with status {}",
                exit_status.unwrap_or_default()
            )
        } else {
            format!(
                "host stats exited with status {}: {detail}",
                exit_status.unwrap_or_default()
            )
        }));
    }

    let text = String::from_utf8_lossy(&stdout);
    Ok(parse_host_stats(&text))
}

fn parse_host_stats(output: &str) -> HostStats {
    let mut stats = HostStats::default();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "hostname" => stats.hostname = non_empty(value),
            "kernel" => stats.kernel = non_empty(value),
            "uptime_seconds" => stats.uptime_seconds = parse_u64(value),
            "load1" => stats.load1 = parse_f64(value),
            "cpu_percent" => stats.cpu_percent = parse_f64(value),
            "mem_total_kb" => stats.mem_total_kb = parse_u64(value),
            "mem_used_kb" => stats.mem_used_kb = parse_u64(value),
            "mem_percent" => stats.mem_percent = parse_f64(value),
            "disk_total_kb" => stats.disk_total_kb = parse_u64(value),
            "disk_used_kb" => stats.disk_used_kb = parse_u64(value),
            "disk_percent" => stats.disk_percent = parse_f64(value),
            _ => {}
        }
    }
    stats
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_probe_output() {
        let stats = parse_host_stats(
            "hostname=prod-01\n\
             kernel=Linux 6.8 x86_64 GNU/Linux\n\
             uptime_seconds=12345\n\
             load1=0.42\n\
             cpu_percent=12.5\n\
             mem_total_kb=16000000\n\
             mem_used_kb=4000000\n\
             mem_percent=25.0\n\
             disk_total_kb=100000000\n\
             disk_used_kb=55000000\n\
             disk_percent=55\n",
        );
        assert_eq!(stats.hostname.as_deref(), Some("prod-01"));
        assert_eq!(stats.uptime_seconds, Some(12345));
        assert_eq!(stats.cpu_percent, Some(12.5));
        assert_eq!(stats.mem_percent, Some(25.0));
        assert_eq!(stats.disk_percent, Some(55.0));
    }

    #[test]
    fn ignores_missing_or_bad_values() {
        let stats = parse_host_stats("hostname=\ncpu_percent=NaN\nmem_total_kb=nope\n");
        assert_eq!(stats.hostname, None);
        assert_eq!(stats.cpu_percent, None);
        assert_eq!(stats.mem_total_kb, None);
    }
}
