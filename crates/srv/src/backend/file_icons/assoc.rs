// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fingerprint of whatever decides which icon a file type has, so a changed
//! default app or icon theme makes a new cache entry instead of needing a
//! notification (srv has no window to receive one). Spec §4.4.

use super::request::{IconRequest, Kind};

/// Join what decides the icon into one string; the cache hashes it.
#[cfg(windows)]
pub fn fingerprint(req: &IconRequest) -> String {
    use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER};
    let reg = |root, path: &str, value: &str| registry_string(root, path, value).unwrap_or_default();
    if req.kind == Kind::Dir {
        return format!("dir|{}", reg(HKEY_CLASSES_ROOT, "Folder\\DefaultIcon", ""));
    }
    let dotted = format!(".{}", req.ext);
    // The extension's own ProgID, and the one the user chose over it (Open with > Always).
    let class = reg(HKEY_CLASSES_ROOT, &dotted, "");
    let chosen = reg(
        HKEY_CURRENT_USER,
        &format!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\{dotted}\\UserChoice"),
        "ProgId",
    );
    let prog = if chosen.is_empty() { class.clone() } else { chosen.clone() };
    let icon = if prog.is_empty() { String::new() } else { reg(HKEY_CLASSES_ROOT, &format!("{prog}\\DefaultIcon"), "") };
    let handler = if prog.is_empty() { String::new() } else { reg(HKEY_CLASSES_ROOT, &format!("{prog}\\ShellEx\\IconHandler"), "") };
    format!("file|{class}|{chosen}|{icon}|{handler}")
}

#[cfg(not(windows))]
pub fn fingerprint(req: &IconRequest) -> String {
    format!("{}|none", req.kind.as_str())
}

/// A string value (the default value when `value` is empty), or `None` when it isn't there.
#[cfg(windows)]
fn registry_string(root: windows::Win32::System::Registry::HKEY, path: &str, value: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (path, value) = (wide(path), wide(value));
    let value_ptr = if value.len() == 1 { PCWSTR::null() } else { PCWSTR(value.as_ptr()) };
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes: u32 = 0;
    // SAFETY: the pointers are to NUL-terminated buffers that outlive the call; the size
    // is asked for first, then the buffer is sized to it.
    unsafe {
        if RegGetValueW(root, PCWSTR(path.as_ptr()), value_ptr, flags, None, None, Some(&mut bytes)).is_err() || bytes == 0 {
            return None;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        if RegGetValueW(root, PCWSTR(path.as_ptr()), value_ptr, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut bytes))
            .is_err()
        {
            return None;
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn req(kind: Kind, ext: &str) -> IconRequest {
        IconRequest { kind, ext: ext.to_string(), px: 16 }
    }

    #[test]
    fn the_fingerprint_reads_the_registry_and_differs_by_type() {
        let txt = fingerprint(&req(Kind::File, "txt"));
        assert!(txt.starts_with("file|"), "{txt}");
        assert_eq!(txt, fingerprint(&req(Kind::File, "txt")), "stable between calls");
        assert!(txt.len() > "file|||| ".len(), "Windows has an association for .txt: {txt}");
        assert_ne!(txt, fingerprint(&req(Kind::File, "nope_zz")));
        assert!(fingerprint(&req(Kind::Dir, "")).starts_with("dir|"));
    }
}
