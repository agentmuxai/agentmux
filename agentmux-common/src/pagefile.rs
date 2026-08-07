// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows page-file volume tracking, shared between `agentmux-srv`
//! (`backend/sysinfo.rs`'s StatusBar telemetry) and `agentmux-cef`
//! (`memory_heartbeat.rs`'s low-memory banner). Extracted from `sysinfo.rs`
//! by `docs/specs/SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07.md` §4 so the
//! two processes can't independently drift on "what counts as system-managed"
//! the way the commit-ratio classifier and the StatusBar gauge once did
//! (issue #2218) — one implementation, two callers.
//!
//! Per `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md` §5.2 P0: a
//! system-managed page file wants to grow toward `min(3×RAM, ⅛ volume)` but
//! can't if the volume it lives on doesn't have the free space. This module
//! answers two questions: which volume backs the page file, and can Windows
//! actually grow it there.

const BYTES_PER_GB: f64 = 1_073_741_824.0;

/// One `PagingFiles` registry entry
/// (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management`),
/// parsed from its `"<path> <initial_mb> <maximum_mb>"` string form. Per
/// [Microsoft's documented format](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/how-to-determine-the-appropriate-page-file-size-for-64-bit-versions-of-windows):
/// `initial_mb == 0 && maximum_mb == 0` means "system managed" (auto-grow)
/// for that specific pagefile.
#[derive(Debug, Clone, PartialEq)]
struct PagingFileEntry {
    /// Drive letter the pagefile lives on, uppercased, e.g. `"C"`. `None` if
    /// the path couldn't be parsed as `<letter>:\...` (UNC path, malformed).
    drive_letter: Option<char>,
    system_managed: bool,
}

/// Parse one raw `"<path> <initial_mb> <maximum_mb>"` line from the
/// `PagingFiles` REG_MULTI_SZ value. Tolerant of the format's use of `??`
/// instead of a drive letter for some `\\?\Volume{guid}\` forms — those
/// yield `drive_letter: None` rather than erroring, since a pure parser
/// should never fail on a value the OS itself produced.
fn parse_paging_file_entry(line: &str) -> Option<PagingFileEntry> {
    let mut parts = line.split_whitespace();
    let path = parts.next()?;
    let initial: u64 = parts.next()?.parse().ok()?;
    let maximum: u64 = parts.next()?.parse().ok()?;
    let drive_letter = path
        .chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase());
    Some(PagingFileEntry {
        drive_letter,
        system_managed: initial == 0 && maximum == 0,
    })
}

/// Decide which volume to watch and whether it's at risk of a stuck
/// (non-growing) page file, from the parsed `PagingFiles` entries.
///
/// - **Empty list** (the common default): Windows fully automatically
///   manages a single pagefile's size AND location — not expressed as any
///   registry entry at all. Falls back to `system_drive` as the practical
///   answer (where CEF/the OS itself lives; the overwhelmingly likely
///   pagefile location in this configuration) — `system_managed: true`.
/// - **Has entries**: watch the first volume with a `system_managed: true`
///   entry (the volume that can actually run out of the growth room this
///   spec is about). If every entry is fixed-size, no volume is at growth
///   risk from disk space — report `system_managed: false` on
///   `system_drive` for visibility, but callers must not apply the
///   "can't grow" risk framing to a fixed-size pagefile.
fn resolve_pagefile_watch_target(entries: &[PagingFileEntry], system_drive: char) -> (char, bool) {
    if entries.is_empty() {
        return (system_drive, true);
    }
    for e in entries {
        if e.system_managed {
            return (e.drive_letter.unwrap_or(system_drive), true);
        }
    }
    (system_drive, false)
}

/// Read `PagingFiles` and split it into entries. Non-Windows / read failure
/// returns an empty vec (caller's `resolve_pagefile_watch_target` then
/// degrades to "watch the system drive as if system-managed" — the correct
/// fail-open default: assume the common case rather than silently reporting
/// nothing).
#[cfg(target_os = "windows")]
fn read_paging_files_registry() -> Vec<PagingFileEntry> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_MULTI_SZ,
    };

    let subkey: Vec<u16> =
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Memory Management\0"
            .encode_utf16()
            .collect();
    let value_name: Vec<u16> = "PagingFiles\0".encode_utf16().collect();

    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), 0, KEY_READ, &mut hkey) as u32
            != ERROR_SUCCESS
        {
            return Vec::new();
        }

        let mut buf_bytes: u32 = 0;
        let mut value_type: u32 = 0;
        let query1 = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut buf_bytes,
        );
        if query1 as u32 != ERROR_SUCCESS || value_type != REG_MULTI_SZ || buf_bytes == 0 {
            RegCloseKey(hkey);
            return Vec::new();
        }

        let mut buf: Vec<u16> = vec![0u16; buf_bytes as usize / 2];
        let query2 = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut u8,
            &mut buf_bytes,
        );
        RegCloseKey(hkey);
        if query2 as u32 != ERROR_SUCCESS {
            return Vec::new();
        }

        // REG_MULTI_SZ: NUL-separated strings, terminated by a double NUL.
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| parse_paging_file_entry(&String::from_utf16_lossy(s)))
            .collect()
    }
}

#[cfg(not(target_os = "windows"))]
fn read_paging_files_registry() -> Vec<PagingFileEntry> {
    Vec::new()
}

/// Free/total space (GB) for a drive letter, e.g. `'C'`. Windows-only;
/// returns `None` on any failure (drive not found, API error) so callers
/// degrade to "no data" rather than a wrong number.
#[cfg(target_os = "windows")]
pub fn drive_free_total_gb(drive_letter: char) -> Option<(f64, f64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root: Vec<u16> = format!("{}:\\\0", drive_letter).encode_utf16().collect();
    let mut free_available: u64 = 0;
    let mut total: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            root.as_ptr(),
            &mut free_available,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    Some((
        free_available as f64 / BYTES_PER_GB,
        total as f64 / BYTES_PER_GB,
    ))
}

/// The `(drive_letter, system_managed)` pagefile watch target, resolved
/// once per process lifetime. `PagingFiles` is machine configuration that
/// requires a reboot to take effect — re-reading the registry on every
/// caller's poll loop would be pure waste. Free disk space, by contrast,
/// genuinely changes during a session — call `drive_free_total_gb` for that,
/// as often as the caller's own poll cadence needs.
#[cfg(target_os = "windows")]
pub fn pagefile_watch_target() -> (char, bool) {
    static TARGET: std::sync::OnceLock<(char, bool)> = std::sync::OnceLock::new();
    *TARGET.get_or_init(|| {
        let system_drive = std::env::var("SystemDrive")
            .ok()
            .and_then(|s| s.chars().next())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('C');
        let entries = read_paging_files_registry();
        resolve_pagefile_watch_target(&entries, system_drive)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_managed_entry() {
        let e = parse_paging_file_entry("C:\\pagefile.sys 0 0").unwrap();
        assert_eq!(e.drive_letter, Some('C'));
        assert!(e.system_managed);
    }

    #[test]
    fn parses_fixed_size_entry() {
        let e = parse_paging_file_entry("D:\\pagefile.sys 4096 8192").unwrap();
        assert_eq!(e.drive_letter, Some('D'));
        assert!(!e.system_managed);
    }

    #[test]
    fn lowercases_drive_letter_normalized_to_upper() {
        // Windows can report either case; the registry doesn't enforce one.
        let e = parse_paging_file_entry("c:\\pagefile.sys 0 0").unwrap();
        assert_eq!(e.drive_letter, Some('C'));
    }

    #[test]
    fn malformed_line_returns_none() {
        assert!(parse_paging_file_entry("").is_none());
        assert!(parse_paging_file_entry("C:\\pagefile.sys").is_none());
        assert!(parse_paging_file_entry("C:\\pagefile.sys notanumber 0").is_none());
    }

    #[test]
    fn empty_entries_falls_back_to_system_drive_as_managed() {
        // The common default: no explicit PagingFiles entries at all means
        // Windows fully auto-manages a single pagefile — not expressed as a
        // registry entry, so an empty list must NOT be read as "no pagefile
        // risk" (the opposite of the truth).
        let (drive, managed) = resolve_pagefile_watch_target(&[], 'C');
        assert_eq!(drive, 'C');
        assert!(managed);
    }

    #[test]
    fn watches_the_first_system_managed_volume() {
        let entries = vec![
            PagingFileEntry { drive_letter: Some('D'), system_managed: false },
            PagingFileEntry { drive_letter: Some('E'), system_managed: true },
        ];
        let (drive, managed) = resolve_pagefile_watch_target(&entries, 'C');
        assert_eq!(drive, 'E');
        assert!(managed);
    }

    #[test]
    fn all_fixed_size_reports_not_managed_on_system_drive() {
        // No volume is at "can't grow" risk — but we still surface a
        // number (system drive) for visibility, correctly flagged as not
        // subject to this specific risk.
        let entries = vec![PagingFileEntry { drive_letter: Some('D'), system_managed: false }];
        let (drive, managed) = resolve_pagefile_watch_target(&entries, 'C');
        assert_eq!(drive, 'C');
        assert!(!managed);
    }
}
