// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS: `proc_listallpids`, then per process:
//! - `PROC_PIDT_SHORTBSDINFO` (parent, name) and `PROC_PIDUNIQIDENTIFIERINFO`
//!   (a unique id that survives PID reuse) — the kernel answers both for any
//!   user's process;
//! - `proc_pid_rusage(RUSAGE_INFO_V2)` for CPU and `ri_phys_footprint`
//!   (Activity Monitor's "Memory"), and `PROC_PIDTBSDINFO` for the start
//!   time — same user only. Another user's process gets `EPERM`, and its CPU
//!   and memory stay `None`: reading them needs root, which this never asks
//!   for (no `task_for_pid`, no helper).
//!
//! CPU times from `proc_pid_rusage` are Mach absolute-time ticks, not
//! nanoseconds: 125/3 ns each on Apple Silicon. They are converted with
//! `mach_timebase_info`; reading them as nanoseconds is the bug htop and
//! osquery both shipped.

use std::ffi::CStr;
use std::mem::{size_of, zeroed};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;

use crate::{base_name, ProcInfo};

/// `<sys/proc_info.h>` `struct proc_bsdshortinfo` — not in `libc`.
#[allow(dead_code)] // the kernel's layout; not every field is read
#[repr(C)]
struct ProcBsdShortInfo {
    pbsi_pid: u32,
    pbsi_ppid: u32,
    pbsi_pgid: u32,
    pbsi_status: u32,
    pbsi_comm: [c_char; 16],
    pbsi_flags: u32,
    pbsi_uid: u32,
    pbsi_gid: u32,
    pbsi_ruid: u32,
    pbsi_rgid: u32,
    pbsi_svuid: u32,
    pbsi_svgid: u32,
    pbsi_rfu: u32,
}
const PROC_PIDT_SHORTBSDINFO: c_int = 13;

/// `struct proc_uniqidentifierinfo` — not in `libc`. Only the leading id is
/// read; the tail is room for what newer kernels append.
#[allow(dead_code)] // the kernel's layout; not every field is read
#[repr(C)]
struct ProcUniqIdentifierInfo {
    p_uuid: [u8; 16],
    p_uniqueid: u64,
    p_puniqueid: u64,
    p_reserve: [u64; 4],
}
const PROC_PIDUNIQIDENTIFIERINFO: c_int = 17;

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}
extern "C" {
    // In libc only as deprecated (it points at the mach2 crate).
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
}

/// Nanoseconds per Mach tick, as a ratio.
fn timebase() -> (u64, u64) {
    static TB: OnceLock<(u64, u64)> = OnceLock::new();
    *TB.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        let ok = unsafe { mach_timebase_info(&mut info) } == 0 && info.numer != 0 && info.denom != 0;
        if ok { (info.numer as u64, info.denom as u64) } else { (1, 1) }
    })
}

fn ticks_to_ns(ticks: u64) -> u64 {
    let (numer, denom) = timebase();
    (ticks as u128 * numer as u128 / denom as u128) as u64
}

/// `proc_pidinfo` into a `T`; `None` unless the kernel filled at least the
/// part of it we know about.
fn pidinfo<T>(pid: u32, flavor: c_int) -> Option<T> {
    let mut v: T = unsafe { zeroed() };
    let n = unsafe { libc::proc_pidinfo(pid as c_int, flavor, 0, (&mut v as *mut T).cast::<c_void>(), size_of::<T>() as c_int) };
    (n > 0).then_some(v)
}

fn all_pids() -> std::io::Result<Vec<u32>> {
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if n <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Room for processes started since the count.
    let mut pids: Vec<c_int> = vec![0; n as usize + 64];
    let n = unsafe {
        libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * size_of::<c_int>()) as c_int)
    };
    if n <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    pids.truncate(n as usize);
    Ok(pids.into_iter().filter(|&p| p > 0).map(|p| p as u32).collect())
}

fn c_name(raw: &[c_char]) -> String {
    let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The executable's file name, from its path (no same-user check), else the
/// 16-character `comm`.
fn exe_name(pid: u32, comm: &[c_char]) -> String {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid as c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n > 0 {
        if let Ok(path) = CStr::from_bytes_until_nul(&buf).map(|c| c.to_string_lossy().into_owned()) {
            if !path.is_empty() {
                return base_name(&path).to_string();
            }
        }
    }
    c_name(comm)
}

pub fn snapshot() -> std::io::Result<Vec<ProcInfo>> {
    let mut out = Vec::with_capacity(512);
    for pid in all_pids()? {
        // Gone already: skip it.
        let Some(short) = pidinfo::<ProcBsdShortInfo>(pid, PROC_PIDT_SHORTBSDINFO) else { continue };
        let unique = pidinfo::<ProcUniqIdentifierInfo>(pid, PROC_PIDUNIQIDENTIFIERINFO).map(|u| u.p_uniqueid);

        let mut usage: libc::rusage_info_v2 = unsafe { zeroed() };
        let measured = unsafe {
            libc::proc_pid_rusage(pid as c_int, libc::RUSAGE_INFO_V2, (&mut usage as *mut libc::rusage_info_v2).cast())
        } == 0;
        let started_at_ms = if measured {
            pidinfo::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO)
                .map(|b| b.pbi_start_tvsec * 1000 + b.pbi_start_tvusec / 1000)
        } else {
            None
        };
        out.push(ProcInfo {
            pid,
            ppid: Some(short.pbsi_ppid).filter(|&p| p != 0),
            start_key: unique.or(started_at_ms).unwrap_or(0),
            started_at_ms,
            name: exe_name(pid, &short.pbsi_comm),
            cpu_ns: measured.then(|| ticks_to_ns(usage.ri_user_time) + ticks_to_ns(usage.ri_system_time)),
            mem_private: measured.then_some(usage.ri_phys_footprint),
            mem_resident: measured.then_some(usage.ri_resident_size),
            mem_commit: None,
        });
    }
    Ok(out)
}

/// `KERN_PROCARGS2`: argc, the executable path, padding, then argv. Same user
/// only.
pub fn command_line(pid: u32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as c_int];
    let mut size: libc::size_t = 0;
    let ok = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) };
    if ok != 0 || size < size_of::<c_int>() {
        return None;
    }
    let mut buf = vec![0u8; size];
    let ok = unsafe {
        libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0)
    };
    if ok != 0 {
        return None;
    }
    buf.truncate(size);
    parse_procargs2(&buf)
}

pub(crate) fn parse_procargs2(buf: &[u8]) -> Option<String> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let rest = &buf[4..];
    // Skip the executable path and the NULs padding it.
    let path_end = rest.iter().position(|&b| b == 0)?;
    let mut i = path_end;
    while rest.get(i) == Some(&0) {
        i += 1;
    }
    let args: Vec<String> = rest[i..]
        .split(|&b| b == 0)
        .take(argc.max(0) as usize)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    Some(args.join(" ")).filter(|s| !s.is_empty())
}

pub fn cpu_count() -> usize {
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n > 0 { n as usize } else { 1 }
}

const _: () = assert!(size_of::<ProcBsdShortInfo>() == 64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procargs2_skips_the_path_and_its_padding() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/node\0\0\0\0node\0/repo/cli.js\0--flag\0HOME=/Users/u\0");
        assert_eq!(parse_procargs2(&buf).as_deref(), Some("node /repo/cli.js --flag"));
        assert_eq!(parse_procargs2(&[1, 0]), None);
    }

    #[test]
    fn another_users_process_is_listed_but_not_measured() {
        // launchd (pid 1) runs as root.
        let all = snapshot().unwrap();
        let launchd = all.iter().find(|p| p.pid == 1).expect("launchd is listed");
        assert!(!launchd.name.is_empty());
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(launchd.cpu_ns, None);
            assert_eq!(launchd.mem_private, None);
        }
    }
}
