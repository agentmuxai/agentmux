// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux: `/proc/<pid>/stat` (parent, CPU times, start time) and
//! `/proc/<pid>/status` (memory). Both are world-readable unless `/proc` is
//! mounted with `hidepid`, in which case other users' processes simply aren't
//! listed. `smaps_rollup` (PSS) is deliberately not read: it walks page
//! tables, too slow for every refresh.

use std::sync::OnceLock;

use crate::ProcInfo;

pub fn snapshot() -> std::io::Result<Vec<ProcInfo>> {
    let hz = clock_ticks();
    let boot_ms = boot_time_ms();
    let mut out = Vec::with_capacity(512);
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        // Gone between the listing and the read: skip it.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(s) = parse_stat(&stat) else { continue };
        // Kernel threads have no user memory. By their flag, not by PID: in a
        // PID namespace (a container, WSL) PID 2 is an ordinary process.
        if s.flags & PF_KTHREAD != 0 {
            continue;
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok();
        let mem = status.as_deref().map(parse_status).unwrap_or_default();
        out.push(ProcInfo {
            pid,
            ppid: Some(s.ppid).filter(|&p| p != 0),
            start_key: s.start_ticks,
            started_at_ms: boot_ms.map(|b| b + s.start_ticks * 1000 / hz),
            name: s.comm,
            cpu_ns: Some((s.utime + s.stime) * (1_000_000_000 / hz)),
            mem_private: mem.private(),
            mem_resident: mem.rss_kb.map(|k| k * 1024),
            mem_commit: None,
        });
    }
    Ok(out)
}

pub fn command_line(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(join_nul_separated(&raw)).filter(|s| !s.is_empty())
}

/// The start time [`snapshot`] reports as `start_key`, read for one process.
pub fn start_key(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_stat(&stat).map(|s| s.start_ticks)
}

pub fn cpu_count() -> usize {
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n > 0 { n as usize } else { 1 }
}

fn clock_ticks() -> u64 {
    static HZ: OnceLock<u64> = OnceLock::new();
    *HZ.get_or_init(|| {
        let n = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if n > 0 { n as u64 } else { 100 }
    })
}

/// `btime` from `/proc/stat`: when the machine booted, unix ms.
fn boot_time_ms() -> Option<u64> {
    static BOOT: OnceLock<Option<u64>> = OnceLock::new();
    *BOOT.get_or_init(|| {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        parse_btime(&stat).map(|s| s * 1000)
    })
}

/// `include/linux/sched.h`: the process is a kernel thread.
const PF_KTHREAD: u64 = 0x0020_0000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stat {
    pub comm: String,
    pub ppid: u32,
    /// Field 9, the kernel's `PF_*` flags.
    pub flags: u64,
    pub utime: u64,
    pub stime: u64,
    pub start_ticks: u64,
}

/// `/proc/<pid>/stat`. The name (field 2) is in parentheses and may itself
/// hold spaces and parentheses, so the fields after it are found from the
/// LAST `)`.
pub(crate) fn parse_stat(s: &str) -> Option<Stat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let comm = s.get(open + 1..close)?.to_string();
    // Field 3 (state) onward.
    let rest: Vec<&str> = s.get(close + 1..)?.split_ascii_whitespace().collect();
    let field = |n: usize| rest.get(n - 3).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        comm,
        ppid: field(4)? as u32,
        flags: field(9)?,
        utime: field(14)?,
        stime: field(15)?,
        start_ticks: field(22)?,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StatusMem {
    pub rss_kb: Option<u64>,
    pub anon_kb: Option<u64>,
    pub swap_kb: Option<u64>,
}

impl StatusMem {
    /// Anonymous resident plus swapped: private memory, the part that grows
    /// when the process leaks (Chrome's "private footprint").
    fn private(&self) -> Option<u64> {
        self.anon_kb.map(|a| (a + self.swap_kb.unwrap_or(0)) * 1024)
    }
}

pub(crate) fn parse_status(s: &str) -> StatusMem {
    let mut m = StatusMem::default();
    for line in s.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let kb = || value.split_ascii_whitespace().next().and_then(|v| v.parse::<u64>().ok());
        match key {
            "VmRSS" => m.rss_kb = kb(),
            "RssAnon" => m.anon_kb = kb(),
            "VmSwap" => m.swap_kb = kb(),
            _ => {}
        }
    }
    m
}

pub(crate) fn parse_btime(stat: &str) -> Option<u64> {
    stat.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())
}

/// argv as one line: NUL-separated, trailing NUL dropped.
pub(crate) fn join_nul_separated(raw: &[u8]) -> String {
    raw.split(|&b| b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_with_a_name_holding_spaces_and_parentheses() {
        let line = "4242 (my (odd) proc) S 4200 4242 4200 0 -1 4194304 500 0 0 0 1234 567 0 0 20 0 3 0 98765 123456789 2048 18446744073709551615";
        assert_eq!(
            parse_stat(line),
            Some(Stat {
                comm: "my (odd) proc".into(),
                ppid: 4200,
                flags: 4_194_304,
                utime: 1234,
                stime: 567,
                start_ticks: 98765
            })
        );
        // kthreadd's own line: PF_KTHREAD (0x200000) is set in field 9.
        let kthreadd = parse_stat("2 (kthreadd) S 0 0 0 0 -1 2129984 0 0 0 0 0 0 0 0 20 0 1 0 2 0 0").unwrap();
        assert_ne!(kthreadd.flags & PF_KTHREAD, 0);
        assert_eq!(parse_stat("12 (truncated) S 1"), None);
    }

    #[test]
    fn status_memory_lines() {
        let status = "Name:\tnode\nVmRSS:\t  204800 kB\nRssAnon:\t  150000 kB\nRssFile:\t   54800 kB\nVmSwap:\t    1000 kB\n";
        let m = parse_status(status);
        assert_eq!(m, StatusMem { rss_kb: Some(204_800), anon_kb: Some(150_000), swap_kb: Some(1000) });
        assert_eq!(m.private(), Some(151_000 * 1024));
        // Kernels before 4.5 have no RssAnon: no private figure rather than a wrong one.
        assert_eq!(parse_status("VmRSS:\t 10 kB\n").private(), None);
    }

    #[test]
    fn btime_and_cmdline() {
        assert_eq!(parse_btime("cpu  1 2 3\nbtime 1700000000\nprocesses 5\n"), Some(1_700_000_000));
        assert_eq!(join_nul_separated(b"/bin/bash\0-c\0npm test\0"), "/bin/bash -c npm test");
        assert_eq!(join_nul_separated(b""), "");
    }
}
