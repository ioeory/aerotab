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
awk '{ printf "load1=%.2f\nload5=%.2f\nload15=%.2f\n", $1, $2, $3 }' /proc/loadavg 2>/dev/null;
if [ -r /etc/os-release ]; then
    awk -F= '/^PRETTY_NAME=/ { value=substr($0,index($0,"=")+1); gsub(/^"|"$/, "", value); printf "os=%s\n", value }' /etc/os-release;
else uname -s 2>/dev/null | awk '{ printf "os=%s\n", $0 }'; fi;
awk '/^SwapTotal:/ { t=$2; found=1 } /^SwapFree:/ { f=$2 } END { if (found) printf "swap_total_kb=%.0f\nswap_used_kb=%.0f\n", t, t-f }' /proc/meminfo 2>/dev/null;
awk -F: 'NR>2 && NF==2 { name=$1; gsub(/^[[:space:]]+|[[:space:]]+$/, "", name); split($2,v," "); printf "network=%s|%s|%s\n", name, v[1], v[9] }' /proc/net/dev 2>/dev/null;
awk 'NF>=14 { printf "disk_io=%s|%.0f|%.0f\n", $3, $6*512, $10*512 }' /proc/diskstats 2>/dev/null;
df -Pk 2>/dev/null | awk 'NR>1 && $2 ~ /^[0-9]+$/ && $5 ~ /^[0-9]+%$/ { pct=$5; sub(/%$/, "", pct); mount=$6; for(i=7;i<=NF;i++) mount=mount " " $i; printf "mount=%s|%s|%s|%s|%s\n", $1, $2, $3, pct, mount }';
if [ ! -r /proc/loadavg ]; then
    sysctl -n vm.loadavg 2>/dev/null | awk '{ for (i=1; i<=NF; i++) { gsub(/[{}]/, "", $i); if ($i ~ /^[0-9.]+$/) { printf "load1=%.2f\n", $i; exit } } }';
fi;
read_cpu() { awk '/^cpu([0-9]+)? / { idle=$5+$6; total=0; for(i=2;i<=9 && i<=NF;i++) total+=$i; printf "%s %.0f %.0f\n", $1, idle, total }' /proc/stat 2>/dev/null; }
cpu_before=$(read_cpu); sleep 0.2;
read_cpu | awk -v before="$cpu_before" 'BEGIN { n=split(before,lines,"\n"); for(i=1;i<=n;i++){ split(lines[i],v," "); idle[v[1]]=v[2]; total[v[1]]=v[3] } } { if(!($1 in total)) next; dt=$3-total[$1]; di=$2-idle[$1]; if(dt>0 && di>=0 && di<=dt){ pct=(1-di/dt)*100; if($1=="cpu") printf "cpu_percent=%.1f\n",pct; else printf "cpu_core=%s|%.1f\n",$1,pct } }';
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CpuCore {
    pub name: String,
    pub percent: f64,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NetworkInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiskIo {
    pub name: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MountUsage {
    pub device: String,
    pub mount_point: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct HostStats {
    pub os: Option<String>,
    pub load5: Option<f64>,
    pub load15: Option<f64>,
    pub swap_total_kb: Option<u64>,
    pub swap_used_kb: Option<u64>,
    pub cpu_cores: Option<Vec<CpuCore>>,
    pub network_interfaces: Option<Vec<NetworkInterface>>,
    pub disk_io: Option<Vec<DiskIo>>,
    pub mounts: Option<Vec<MountUsage>>,
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
    timeout(
        Duration::from_secs(20),
        probe_host_stats_inner(profile, known_hosts),
    )
    .await
    .map_err(|_| SshError::Channel("host stats connection timed out".into()))?
}

async fn probe_host_stats_inner(
    profile: &SshProfile,
    known_hosts: Option<KnownHosts>,
) -> Result<HostStats, SshError> {
    let handle =
        connect_authenticated(profile, known_hosts, SshTransportSettings::default()).await?;
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
                ChannelMsg::Data { ref data } => {
                    if stdout.len().saturating_add(data.len()) > 1_048_576 {
                        return Err(SshError::Channel("host stats output exceeded limit".into()));
                    }
                    stdout.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext: _ } if stderr.len() < 8192 => {
                    stderr.extend_from_slice(&data[..data.len().min(8192 - stderr.len())]);
                }
                ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok::<_, SshError>((stdout, stderr, exit_status))
    };
    let collected = timeout(Duration::from_secs(10), collect).await;
    // Close even when collection fails, times out or exceeds the output bound.
    let _ = timeout(
        Duration::from_secs(1),
        handle.disconnect(Disconnect::ByApplication, "host stats complete", "en"),
    )
    .await;
    let (stdout, stderr, exit_status) =
        collected.map_err(|_| SshError::Channel("host stats timed out".into()))??;

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
            "os" => stats.os = non_empty(value),
            "load5" => stats.load5 = parse_f64(value),
            "load15" => stats.load15 = parse_f64(value),
            "swap_total_kb" => stats.swap_total_kb = parse_u64(value),
            "swap_used_kb" => stats.swap_used_kb = parse_u64(value),
            "cpu_core" => {
                if let Some((name, percent)) = value.split_once('|') {
                    if let Some(percent) = parse_f64(percent).filter(|v| (0.0..=100.0).contains(v))
                    {
                        stats.cpu_cores.get_or_insert_with(Vec::new).push(CpuCore {
                            name: name.into(),
                            percent,
                        });
                    }
                }
            }
            "network" | "disk_io" => {
                let fields: Vec<_> = value.split('|').collect();
                if fields.len() == 3 {
                    if let (Some(a), Some(b)) = (parse_u64(fields[1]), parse_u64(fields[2])) {
                        if key.trim() == "network" {
                            stats.network_interfaces.get_or_insert_with(Vec::new).push(
                                NetworkInterface {
                                    name: fields[0].into(),
                                    rx_bytes: a,
                                    tx_bytes: b,
                                },
                            );
                        } else {
                            stats.disk_io.get_or_insert_with(Vec::new).push(DiskIo {
                                name: fields[0].into(),
                                read_bytes: a,
                                write_bytes: b,
                            });
                        }
                    }
                }
            }
            "mount" => {
                let fields: Vec<_> = value.splitn(5, '|').collect();
                if fields.len() == 5 {
                    if let (Some(total_kb), Some(used_kb), Some(percent)) = (
                        parse_u64(fields[1]),
                        parse_u64(fields[2]),
                        parse_f64(fields[3]),
                    ) {
                        stats.mounts.get_or_insert_with(Vec::new).push(MountUsage {
                            device: fields[0].into(),
                            mount_point: fields[4].into(),
                            total_kb,
                            used_kb,
                            percent,
                        });
                    }
                }
            }
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
    fn parses_extended_linux_metrics_and_keeps_missing_values_absent() {
        let stats = parse_host_stats("os=Debian GNU/Linux 13\nload5=1.25\nload15=0.75\nswap_total_kb=2048\nswap_used_kb=512\ncpu_core=cpu0|12.5\ncpu_core=cpu1|25\nnetwork=eth0|123456|98765\ndisk_io=sda|1024|4096\nmount=/dev/sda1|100000|50000|50|/mnt/data disk\n");
        assert_eq!(stats.os.as_deref(), Some("Debian GNU/Linux 13"));
        assert_eq!(stats.load5, Some(1.25));
        assert_eq!(stats.load15, Some(0.75));
        assert_eq!(stats.swap_used_kb, Some(512));
        assert_eq!(stats.cpu_cores.as_ref().unwrap()[1].percent, 25.0);
        assert_eq!(
            stats.network_interfaces.as_ref().unwrap()[0].rx_bytes,
            123456
        );
        assert_eq!(stats.disk_io.as_ref().unwrap()[0].write_bytes, 4096);
        assert_eq!(
            stats.mounts.as_ref().unwrap()[0].mount_point,
            "/mnt/data disk"
        );
        let absent = parse_host_stats("hostname=bsd\nnetwork=bad|nope|1\ncpu_core=cpu0|NaN\n");
        assert!(absent.network_interfaces.is_none());
        assert!(absent.cpu_cores.is_none());
        assert!(absent.swap_total_kb.is_none());
    }

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
