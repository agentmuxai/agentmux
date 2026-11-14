// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The version of an application on this computer, found from the executable names a computer-use skill lists
//! (docs/specs/SPEC_PACKS_2026_11_05.md §8: "a lookup per row, no inventory scan"). "Not found" is a normal answer.
//!
//! - **Windows:** the App Paths registry key, then `PATH`, then the folders directly under Program Files and
//!   `%LOCALAPPDATA%\Programs` (one level further down); the version is the executable's file version.
//! - **macOS:** the `.app` bundles directly in `/Applications` and `~/Applications` that hold `Contents/MacOS/<name>`;
//!   the version is the bundle's `CFBundleShortVersionString`.
//! - **Linux:** `PATH` only. The app is found, its version is not read.

use std::path::PathBuf;
#[cfg(any(windows, target_os = "macos"))]
use std::path::Path;

/// An application found here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What the executable or bundle says it is; `None` when it says nothing readable.
    pub version: Option<String>,
}

/// The first of `executables` that is installed here, with its version.
pub fn find(executables: &[String]) -> Option<Found> {
    executables.iter().filter(|e| !e.trim().is_empty()).find_map(|name| locate(name.trim()))
}

/// `a.b.c.d` from a Windows file version, without trailing zero parts beyond the first two (`16.0.0.0` is `16.0`).
#[cfg(any(windows, test))]
fn dotted(parts: [u16; 4]) -> String {
    let keep = parts.iter().rposition(|p| *p != 0).map_or(2, |i| (i + 1).max(2));
    parts[..keep].iter().map(u16::to_string).collect::<Vec<_>>().join(".")
}

/// The value of `key` in an XML property list (the text of the `<string>` that follows `<key>key</key>`).
#[cfg(any(target_os = "macos", test))]
fn plist_string(xml: &str, key: &str) -> Option<String> {
    let after = &xml[xml.find(&format!("<key>{key}</key>"))? + key.len() + 11..];
    let rest = after.trim_start().strip_prefix("<string>")?;
    let value = rest[..rest.find("</string>")?].trim();
    (!value.is_empty() && value.len() <= 64).then(|| value.to_string())
}

/// `name` in the folders of `path_var`, as a file.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn on_path(name: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var).map(|d| d.join(name)).find(|p| p.is_file())
}

#[cfg(windows)]
fn locate(name: &str) -> Option<Found> {
    let name = if name.contains('.') { name.to_string() } else { format!("{name}.exe") };
    let path = app_paths(&name)
        .or_else(|| std::env::var_os("PATH").and_then(|p| on_path(&name, &p)))
        .or_else(|| in_install_folders(&name))?;
    Some(Found { version: file_version(&path) })
}

/// The path Windows registers for the program, if it registered one.
#[cfg(windows)]
fn app_paths(name: &str) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let key = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{name}");
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(|root| crate::backend::file_icons::registry_string(root, &key, ""))
        .map(|v| PathBuf::from(v.trim().trim_matches('"')))
        .find(|p| p.is_file())
}

/// `<root>/<app>/<name>` and `<root>/<vendor>/<app>/<name>` for the usual install roots.
#[cfg(windows)]
fn in_install_folders(name: &str) -> Option<PathBuf> {
    let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .chain(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs")))
        .collect();
    roots.iter().find_map(|root| {
        let dirs = subfolders(root);
        dirs.iter()
            .map(|d| d.join(name))
            .find(|p| p.is_file())
            .or_else(|| dirs.iter().flat_map(|d| subfolders(d)).map(|d| d.join(name)).find(|p| p.is_file()))
    })
}

#[cfg(windows)]
fn subfolders(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

/// The executable's file version (the fixed part of its version resource).
#[cfg(windows)]
fn file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the calls; the buffer is the size Windows asked for; the pointer
    // VerQueryValueW returns points into that buffer and is read before the buffer is dropped, as an unaligned read.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, buf.as_mut_ptr().cast()).ok()?;
        let mut at: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr().cast(), w!("\\"), &mut at, &mut len).as_bool() || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        let info = std::ptr::read_unaligned(at as *const VS_FIXEDFILEINFO);
        let ms = info.dwFileVersionMS;
        let ls = info.dwFileVersionLS;
        let parts = [(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16];
        (parts != [0; 4]).then(|| dotted(parts))
    }
}

#[cfg(target_os = "macos")]
fn locate(name: &str) -> Option<Found> {
    let homes = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Applications"));
    let bundle = [PathBuf::from("/Applications")]
        .into_iter()
        .chain(homes)
        .flat_map(|root: PathBuf| std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "app"))
        .find(|p| p.join("Contents/MacOS").join(name).is_file())?;
    Some(Found { version: bundle_version(&bundle) })
}

/// A bundle's version from its `Info.plist`: read as XML, or through `plutil` when it is the binary form.
#[cfg(target_os = "macos")]
fn bundle_version(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents/Info.plist");
    let raw = std::fs::read(&plist).ok()?;
    if let Ok(xml) = std::str::from_utf8(&raw) {
        if xml.trim_start().starts_with("<?xml") {
            return plist_string(xml, "CFBundleShortVersionString").or_else(|| plist_string(xml, "CFBundleVersion"));
        }
    }
    let out = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()?;
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !v.is_empty() && v.len() <= 64).then_some(v)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn locate(name: &str) -> Option<Found> {
    std::env::var_os("PATH").and_then(|p| on_path(name, &p)).map(|_| Found { version: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_file_version_drops_trailing_zero_parts_but_keeps_two() {
        assert_eq!(dotted([16, 0, 18227, 20162]), "16.0.18227.20162");
        assert_eq!(dotted([12, 2, 0, 0]), "12.2");
        assert_eq!(dotted([21, 0, 1, 0]), "21.0.1");
        assert_eq!(dotted([16, 0, 0, 0]), "16.0");
        assert_eq!(dotted([7, 0, 0, 0]), "7.0");
    }

    #[test]
    fn a_plist_string_is_read_by_its_key_and_only_when_it_is_a_string() {
        let xml = "<?xml version=\"1.0\"?><plist><dict>\n<key>CFBundleName</key>\n<string>Live</string>\n\
                   <key>CFBundleShortVersionString</key>\n\t<string>12.2.1</string>\n<key>CFBundleVersion</key><integer>5</integer></dict></plist>";
        assert_eq!(plist_string(xml, "CFBundleShortVersionString").as_deref(), Some("12.2.1"));
        assert_eq!(plist_string(xml, "CFBundleName").as_deref(), Some("Live"));
        assert_eq!(plist_string(xml, "CFBundleVersion"), None, "an integer is not a string");
        assert_eq!(plist_string(xml, "Missing"), None);
        assert_eq!(plist_string("<key>K</key><string> </string>", "K"), None, "blank is nothing");
    }

    #[test]
    fn a_name_is_looked_for_in_each_folder_of_the_path_and_must_be_a_file() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("tool"), b"x").unwrap();
        std::fs::create_dir(a.path().join("dir-not-file")).unwrap();
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(on_path("tool", &path), Some(b.path().join("tool")));
        assert_eq!(on_path("dir-not-file", &path), None);
        assert_eq!(on_path("absent", &path), None);
    }

    #[test]
    fn blank_names_find_nothing_and_an_app_that_is_not_here_is_not_found() {
        assert_eq!(find(&[]), None);
        assert_eq!(find(&["  ".to_string()]), None);
        assert_eq!(find(&["agentmux-no-such-app-zz.exe".to_string(), "AgentMuxNoSuchAppZz".to_string()]), None);
    }

    /// notepad.exe is in System32, which is on PATH, and has a file version.
    #[cfg(windows)]
    #[test]
    fn a_windows_program_on_the_path_is_found_with_its_file_version() {
        let found = find(&["notepad.exe".to_string()]).expect("notepad is installed");
        let v = found.version.expect("notepad has a version resource");
        assert!(v.split('.').count() >= 2 && v.split('.').all(|p| p.parse::<u16>().is_ok()), "{v}");
        assert_eq!(find(&["Nope.exe".to_string(), "notepad".to_string()]).map(|f| f.version.is_some()), Some(true), "the name may omit .exe, and the first found wins");
    }

    /// `sh` is on PATH on any Unix; the app is found and no version is read.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_linux_program_on_the_path_is_found_without_a_version() {
        assert_eq!(find(&["sh".to_string()]), Some(Found { version: None }));
    }
}
