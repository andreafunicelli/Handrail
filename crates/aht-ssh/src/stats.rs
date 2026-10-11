//! A server's load (CPU, memory, disk, load average, uptime), read from Linux's /proc with one
//! command on its own channel: nothing shows up in the user's terminal.

use crate::client::SshConnection;
use aht_core::Result;

/// Values the server did not give (another OS, an unreadable file) are `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerStats {
    /// Percent busy over all cores, over [`SAMPLE`].
    pub cpu: Option<f32>,
    pub cores: Option<u32>,
    /// Bytes.
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    /// Bytes, of the root file system.
    pub disk_used: Option<u64>,
    pub disk_total: Option<u64>,
    pub load1: Option<f32>,
    pub uptime_s: Option<u64>,
}

/// Each line is tagged so a missing file only loses its own value. CPU use is the difference
/// between two reads of /proc/stat half a second apart. Run by `sh -c '...'` (whatever the login
/// shell is), so it holds no single quotes.
const SCRIPT: &str = "export LC_ALL=C; \
    echo \"@cpu $(head -n1 /proc/stat 2>/dev/null)\"; sleep 0.5; \
    echo \"@cpu $(head -n1 /proc/stat 2>/dev/null)\"; \
    grep -E \"^(MemTotal|MemAvailable):\" /proc/meminfo 2>/dev/null | sed \"s/^/@mem /\"; \
    echo \"@load $(cat /proc/loadavg 2>/dev/null)\"; \
    echo \"@up $(cat /proc/uptime 2>/dev/null)\"; \
    echo \"@cores $(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null)\"; \
    echo \"@disk $(df -kP / 2>/dev/null | tail -n1)\"";

impl SshConnection {
    /// Sample the server's load (takes about half a second).
    pub async fn stats(&self) -> Result<ServerStats> {
        let out = self.exec(&format!("sh -c '{SCRIPT}'")).await?;
        Ok(ServerStats::parse(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// (busy, total) jiffies from a `cpu ...` line of /proc/stat.
fn cpu_times(line: &str) -> Option<(u64, u64)> {
    let mut f = line.split_whitespace();
    if f.next()? != "cpu" {
        return None;
    }
    let v: Vec<u64> = f.map_while(|x| x.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already in user).
    let total: u64 = v.iter().take(8).sum();
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

impl ServerStats {
    /// Parse the output of [`SCRIPT`].
    pub fn parse(out: &str) -> Self {
        let mut s = ServerStats::default();
        let mut cpu = Vec::new();
        let (mut mem_total, mut mem_avail) = (None, None);
        for line in out.lines() {
            let Some((tag, rest)) = line.split_once(' ') else {
                continue;
            };
            let rest = rest.trim();
            match tag {
                "@cpu" => cpu.extend(cpu_times(rest)),
                "@mem" => {
                    let mut f = rest.split_whitespace();
                    let (key, kb) = (f.next(), f.next().and_then(|v| v.parse::<u64>().ok()));
                    match key {
                        Some("MemTotal:") => mem_total = kb.map(|k| k * 1024),
                        Some("MemAvailable:") => mem_avail = kb.map(|k| k * 1024),
                        _ => {}
                    }
                }
                "@load" => s.load1 = rest.split_whitespace().next().and_then(|v| v.parse().ok()),
                "@up" => {
                    s.uptime_s = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                        .map(|v| v as u64)
                }
                "@cores" => s.cores = rest.parse().ok(),
                "@disk" => {
                    // Filesystem 1024-blocks Used Available Capacity Mounted-on
                    let f: Vec<&str> = rest.split_whitespace().collect();
                    if f.len() >= 4 {
                        let kb = |i: usize| f[i].parse::<u64>().ok().map(|k| k * 1024);
                        s.disk_total = kb(1);
                        s.disk_used = kb(2);
                    }
                }
                _ => {}
            }
        }
        if let [(b0, t0), (b1, t1)] = cpu.as_slice() {
            if t1 > t0 {
                let pct = b1.saturating_sub(*b0) as f32 * 100.0 / (t1 - t0) as f32;
                s.cpu = Some(pct.clamp(0.0, 100.0));
            }
        }
        s.mem_total = mem_total;
        s.mem_used = mem_total.zip(mem_avail).map(|(t, a)| t.saturating_sub(a));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_linux_server() {
        let out = "\
@cpu cpu  100 0 100 800 0 0 0 0 0 0
@cpu cpu  150 0 150 850 0 0 0 0 0 0
@mem MemTotal:        8000000 kB
@mem MemAvailable:    6000000 kB
@load 0.42 0.30 0.25 1/123 4567
@up 273600.55 1000000.00
@cores 4
@disk /dev/sda1 41152736 25000000 14000000 65% /
";
        let s = ServerStats::parse(out);
        // 100 busy jiffies out of 150.
        assert!((s.cpu.unwrap() - 66.666).abs() < 0.01, "{:?}", s.cpu);
        assert_eq!(s.mem_total, Some(8_000_000 * 1024));
        assert_eq!(s.mem_used, Some(2_000_000 * 1024));
        assert_eq!(s.load1, Some(0.42));
        assert_eq!(s.uptime_s, Some(273_600));
        assert_eq!(s.cores, Some(4));
        assert_eq!(s.disk_total, Some(41_152_736 * 1024));
        assert_eq!(s.disk_used, Some(25_000_000 * 1024));
    }

    #[test]
    fn the_script_survives_sh_c_quoting() {
        assert!(!SCRIPT.contains('\''));
    }

    #[test]
    fn missing_values_stay_none() {
        // macOS: no /proc; df still answers.
        let out = "@cpu \n@cpu \n@load \n@up \n@cores 8\n@disk /dev/disk3s1 1000 400 600 40% /\n";
        let s = ServerStats::parse(out);
        assert_eq!(s.cpu, None);
        assert_eq!(s.mem_used, None);
        assert_eq!(s.load1, None);
        assert_eq!(s.uptime_s, None);
        assert_eq!(s.cores, Some(8));
        assert_eq!(s.disk_used, Some(400 * 1024));
        assert_eq!(ServerStats::parse(""), ServerStats::default());
    }
}
