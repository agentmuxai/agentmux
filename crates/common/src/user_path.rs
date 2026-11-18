// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's own PATH on Windows (`HKEY_CURRENT_USER\Environment`, value `Path`): what `install.ps1 -Headless` adds
//! one entry to, and `agentmux uninstall` takes that entry back out of. Only the person's own list is read or written,
//! never the machine's, and only entries inside the install folder are removed; the rest of the value is kept as it is.
//! The text work is plain code that is tested on every platform.

use std::path::Path;

/// An entry as it is compared: a `%LOCALAPPDATA%` or `%USERPROFILE%` written in it is replaced by the value in `vars`,
/// quotes and a closing `\` or `/` are dropped, `/` becomes `\`, and the case does not matter.
fn normalized(entry: &str, vars: &[(&str, &str)]) -> String {
    let mut text = entry.trim().trim_matches('"').to_string();
    for (name, value) in vars {
        let marker = format!("%{}%", name.to_ascii_lowercase());
        while let Some(at) = text.to_ascii_lowercase().find(&marker) {
            text.replace_range(at..at + marker.len(), value);
        }
    }
    text.replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
}

/// `path_value` without the entries that are `base` or inside it, or `None` when none are (so nothing is written). An
/// empty `base` removes nothing.
pub fn without_under(path_value: &str, base: &Path, vars: &[(&str, &str)]) -> Option<String> {
    let base = normalized(&base.to_string_lossy(), &[]);
    if base.is_empty() {
        return None;
    }
    let inside = |entry: &str| {
        let entry = normalized(entry, vars);
        entry == base || entry.strip_prefix(base.as_str()).is_some_and(|rest| rest.starts_with('\\'))
    };
    let entries: Vec<&str> = path_value.split(';').collect();
    let kept: Vec<&str> = entries.iter().copied().filter(|e| !inside(e)).collect();
    (kept.len() != entries.len()).then(|| kept.join(";"))
}

/// Take every entry inside `base` out of the person's own PATH. `Ok(true)` when the value was changed. The value keeps
/// its type (an expandable one stays expandable). New programs started from a window that is already open do not see
/// the change until they are started again; nothing is broadcast.
#[cfg(windows)]
pub fn remove_under_from_user_path(base: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};

    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let name: Vec<u16> = "Path\0".encode_utf16().collect();
    // SAFETY: NUL-terminated names that outlive the calls, a buffer as long as the call asked for, and the key closed on
    // every path.
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key) as u32 != ERROR_SUCCESS {
            return Err("cannot open your user environment to read PATH".to_string());
        }
        let result = (|| {
            let (mut kind, mut bytes) = (0u32, 0u32);
            let first = RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut bytes);
            if first as u32 != ERROR_SUCCESS || bytes == 0 {
                return Ok(false);
            }
            let mut buf: Vec<u16> = vec![0u16; (bytes as usize).div_ceil(2)];
            let second = RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr().cast(), &mut bytes);
            if second as u32 != ERROR_SUCCESS {
                return Err("cannot read your user PATH".to_string());
            }
            let len = (bytes as usize / 2).min(buf.len());
            let text = String::from_utf16_lossy(&buf[..len]);
            let text = text.trim_end_matches('\0');
            let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
            let profile = std::env::var("USERPROFILE").unwrap_or_default();
            let Some(new) = without_under(text, base, &[("localappdata", local.as_str()), ("userprofile", profile.as_str())]) else { return Ok(false) };
            let wide: Vec<u16> = new.encode_utf16().chain(std::iter::once(0)).collect();
            let set = RegSetValueExW(key, name.as_ptr(), 0, kind, wide.as_ptr().cast(), (wide.len() * 2) as u32);
            if set as u32 != ERROR_SUCCESS {
                return Err("cannot write your user PATH".to_string());
            }
            Ok(true)
        })();
        RegCloseKey(key);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r"C:\Users\sam\AppData\Local\AgentMux\Headless";

    #[test]
    fn only_the_entries_inside_the_install_folder_are_taken_out() {
        let vars = [("localappdata", r"C:\Users\sam\AppData\Local"), ("userprofile", r"C:\Users\sam")];
        let value = r"C:\Windows;C:\Users\sam\AppData\Local\AgentMux\Headless\current\bin;%LOCALAPPDATA%\AgentMux\Headless\0.60.30\bin;C:\tools";
        assert_eq!(without_under(value, Path::new(BASE), &vars).as_deref(), Some(r"C:\Windows;C:\tools"));
        // Case, quotes, `/` and a closing `\` do not matter.
        assert_eq!(without_under(r#"C:\a;"c:/users/SAM/appdata/local/agentmux/headless/current/bin\";C:\b"#, Path::new(BASE), &vars).as_deref(), Some(r"C:\a;C:\b"));
    }

    #[test]
    fn a_folder_that_only_starts_with_the_same_letters_is_not_inside() {
        let vars = [];
        let value = r"C:\Users\sam\AppData\Local\AgentMux\HeadlessTools\bin;C:\Users\sam\AppData\Local\AgentMux";
        assert_eq!(without_under(value, Path::new(BASE), &vars), None);
    }

    #[test]
    fn nothing_is_written_when_nothing_is_ours_and_an_empty_base_takes_nothing() {
        assert_eq!(without_under(r"C:\Windows;C:\tools", Path::new(BASE), &[]), None);
        assert_eq!(without_under(r"C:\Windows;;C:\tools", Path::new(""), &[]), None);
        assert_eq!(without_under("", Path::new(BASE), &[]), None);
        // The entries that stay keep their order and their own text, empty ones too.
        assert_eq!(without_under(r"C:\a;;%X%\b;C:\Users\sam\AppData\Local\AgentMux\Headless\current\bin", Path::new(BASE), &[]).as_deref(), Some(r"C:\a;;%X%\b"));
    }
}
