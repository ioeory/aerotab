//! Lightweight remote host statistics for SSH sessions.
//!
//! The probe opens a short-lived exec channel instead of writing commands into
//! the user's interactive terminal, so sampling never pollutes scrollback.

use std::time::Duration;

use russh::{ChannelMsg, Disconnect};
use serde::Serialize;
use tokio::time::timeout;

use super::known_hosts::KnownHosts;
use super::{connect_authenticated, SshError, SshProfile, SshTransportSettings};

const HOST_STATS_COMMAND: &str = r#"LC_ALL=C; export LC_ALL;
host=$(hostname 2>/dev/null || uname -n 2>/dev/null || echo ""); [ -n "$host" ] && printf 'hostname=%s\n' "$host";
kernel=$(uname -srmo 2>/dev/null || uname -a 2>/dev/null || echo ""); [ -n "$kernel" ] && printf 'kernel=%s\n' "$kernel";
awk '{ printf "uptime_seconds=%d\n", $1 }' /proc/uptime 2>/dev/null;
if [ ! -r /proc/uptime ]; then
    boot_sec=$(sysctl -n kern.boottime 2>/dev/null | sed -n 's/.*sec = \([0-9][0-9]*\).*/\1/p');
    now_sec=$(date +%s 2>/dev/null || echo "");
    if [ -n "$boot_sec" ] && [ -n "$now_sec" ]; then awk -v boot="$boot_sec" -v now="$now_sec" 'BEGIN { if (now > boot) printf "uptime_seconds=%d\n", now - boot }'; fi;
fi;
awk '{ printf "load1=%.2f\n", $1 }' /proc/loadavg 2>/dev/null;
if [ ! -r /proc/loadavg ]; then
    sysctl -n vm.loadavg 2>/dev/null | awk '{ for (i=1; i<=NF; i++) { gsub(/[{}]/, "", $i); if ($i ~ /^[0-9.]+$/) { printf "load1=%.2f\n", $i; exit } } }';
fi;
read_cpu() { awk '/^cpu / { idle=$5+$6; total=0; for (i=2; i<=NF; i++) total += $i; printf "%s %s\n", idle, total; exit }' /proc/stat 2>/dev/null; }
set -- $(read_cpu); idle1=$1; total1=$2; sleep 0.2; set -- $(read_cpu); idle2=$1; total2=$2;
if [ -n "$total1" ] && [ -n "$total2" ]; then awk -v i1="$idle1" -v t1="$total1" -v i2="$idle2" -v t2="$total2" 'BEGIN { dt=t2-t1; di=i2-i1; if (dt > 0) printf "cpu_percent=%.1f\n", (1 - di / dt) * 100 }'; fi;
awk '/^MemTotal:/ { t=$2 } /^MemAvailable:/ { a=$2 } END { if (t > 0) { used=t-a; printf "mem_total_kb=%d\nmem_used_kb=%d\nmem_percent=%.1f\n", t, used, used * 100 / t } }' /proc/meminfo 2>/dev/null;
if [ ! -r /proc/meminfo ]; then
    page_size=$(pagesize 2>/dev/null || getconf PAGESIZE 2>/dev/null || echo "");
    mem_bytes=$(sysctl -n hw.memsize 2>/dev/null || sysctl -n hw.physmem 2>/dev/null || echo "");
    if [ -n "$page_size" ] && [ -n "$mem_bytes" ]; then
        vm_text=$(vm_stat 2>/dev/null || true);
        vm_free=$(printf '%s\n' "$vm_text" | awk '/Pages free/ { gsub(/\./, "", $3); print $3; exit }');
        vm_spec=$(printf '%s\n' "$vm_text" | awk '/Pages speculative/ { gsub(/\./, "", $3); print $3; exit }');
        if [ -n "$vm_free" ]; then
            awk -v total="$mem_bytes" -v page="$page_size" -v free="$vm_free" -v spec="${vm_spec:-0}" 'BEGIN { free_bytes=(free+spec)*page; used=total-free_bytes; if (total > 0 && used >= 0) printf "mem_total_kb=%d\nmem_used_kb=%d\nmem_percent=%.1f\n", total/1024, used/1024, used*100/total }';
        else
            free_count=$(sysctl -n vm.stats.vm.v_free_count 2>/dev/null || echo "");
            inactive_count=$(sysctl -n vm.stats.vm.v_inactive_count 2>/dev/null || echo "0");
            if [ -n "$free_count" ]; then awk -v total="$mem_bytes" -v page="$page_size" -v free="$free_count" -v inactive="${inactive_count:-0}" 'BEGIN { free_bytes=(free+inactive)*page; used=total-free_bytes; if (total > 0 && used >= 0) printf "mem_total_kb=%d\nmem_used_kb=%d\nmem_percent=%.1f\n", total/1024, used/1024, used*100/total }'; fi;
        fi;
    fi;
fi;
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
    let handle = connect_authenticated(profile, known_hosts, SshTransportSettings::default()).await?;
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

    #[test]
    fn parses_non_linux_fallback_output() {
        let stats = parse_host_stats(
            "hostname=mac-build-01\n\
             kernel=Darwin 23.5.0 arm64\n\
             uptime_seconds=6789\n\
             load1=1.25\n\
             mem_total_kb=16777216\n\
             mem_used_kb=8388608\n\
             mem_percent=50.0\n\
             disk_total_kb=488000000\n\
             disk_used_kb=244000000\n\
             disk_percent=50\n",
        );
        assert_eq!(stats.hostname.as_deref(), Some("mac-build-01"));
        assert_eq!(stats.kernel.as_deref(), Some("Darwin 23.5.0 arm64"));
        assert_eq!(stats.uptime_seconds, Some(6789));
        assert_eq!(stats.load1, Some(1.25));
        assert_eq!(stats.mem_total_kb, Some(16_777_216));
        assert_eq!(stats.mem_used_kb, Some(8_388_608));
        assert_eq!(stats.disk_percent, Some(50.0));
    }
}
