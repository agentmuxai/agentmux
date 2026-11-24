// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dragging files out of AgentMux to other apps (docs/reports/REPORT_DRAG_FILES_OUT_OF_AGENTMUX_2026_11_17.md §5).
//!
//! The app's own UI registers the files a row or an image view stands for before the drag starts and gets a nonce;
//! the drag carries only the nonce, as one custom data entry named [`MARKER`]. When the operating system's drag
//! starts, the host reads the nonce back from the drag's data, takes the registered files once, and adds them to the
//! drag in the form the platform's apps read (a file list on Windows). A drag with no valid nonce is left alone: a page in
//! a Browser pane cannot register files, so it cannot make AgentMux attach local files to its own drag.
//!
//! This file holds what is the same on every platform: the registry, the checks on a path, the reading of the nonce
//! out of Chromium's custom data, and the Windows file list's bytes (a plain byte layout, so it is tested everywhere).

#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The custom data type the page sets on its drag, holding the nonce and nothing else.
pub const MARKER: &str = "application/x-agentmux-file-drag";
/// How long a registration lasts: long enough to press, move and start a drag, short enough that a forgotten one is gone.
const NONCE_TTL: Duration = Duration::from_secs(10);
/// The most files one drag carries.
const MAX_FILES: usize = 100;

struct Entry {
    at: Instant,
    paths: Vec<PathBuf>,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why a path cannot be dragged out, or the file to drag. A file the person can read, that is not a folder, and that is
/// not in a vault folder (those files are never handed to another app).
fn checked(path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("{path:?} is not an absolute path"));
    }
    let real = std::fs::canonicalize(p).map_err(|e| format!("{path:?} can't be read: {e}"))?;
    let meta = std::fs::metadata(&real).map_err(|e| format!("{path:?} can't be read: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path:?} is not a file"));
    }
    if real.ancestors().skip(1).any(|a| a.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("vault"))) {
        return Err("a file in a vault folder is never dragged out".to_string());
    }
    std::fs::File::open(&real).map_err(|e| format!("{path:?} can't be opened: {e}"))?;
    Ok(real)
}

/// Register the files of a drag about to start; the nonce the drag carries. Every path is checked here, once: a drag
/// with a path that fails registers nothing.
pub fn register(paths: &[String]) -> Result<String, String> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(format!("a drag carries 1 to {MAX_FILES} files"));
    }
    let files = paths.iter().map(|p| checked(p)).collect::<Result<Vec<_>, _>>()?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let mut map = lock();
    map.retain(|_, e| e.at.elapsed() <= NONCE_TTL);
    map.insert(nonce.clone(), Entry { at: Instant::now(), paths: files });
    Ok(nonce)
}

/// The files registered under `nonce`, once: a nonce is used up by the drag that carries it, and ends after its
/// lifetime. Anything else (an unknown or used-up nonce, a page's own text) gives nothing.
pub fn take(nonce: &str) -> Option<Vec<PathBuf>> {
    let mut map = lock();
    map.retain(|_, e| e.at.elapsed() <= NONCE_TTL);
    map.remove(nonce).map(|e| e.paths)
}

/// The nonce in Chromium's custom data for a drag (`ui::WriteCustomDataToPickle`): a pickle, a payload size then a
/// count of entries, each a type and a value as UTF-16 strings (a length in characters, the characters, padding to four
/// bytes). `None` for data that does not read as that, or has no entry named [`MARKER`].
pub fn marker_in(pickle: &[u8]) -> Option<String> {
    let u32_at = |at: usize| pickle.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let payload = u32_at(0)?;
    let end = 4usize.checked_add(payload)?;
    if end > pickle.len() {
        return None;
    }
    let mut at = 4;
    let string = |at: &mut usize| -> Option<String> {
        let chars = u32_at(*at)?;
        let bytes = chars.checked_mul(2)?;
        let start = *at + 4;
        let stop = start.checked_add(bytes)?;
        if stop > end {
            return None;
        }
        let units: Vec<u16> = pickle[start..stop].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        *at = stop.checked_add(3)? & !3;
        String::from_utf16(&units).ok()
    };
    let count = u32_at(at)?;
    at += 4;
    for _ in 0..count.min(64) {
        let key = string(&mut at)?;
        let value = string(&mut at)?;
        if key == MARKER {
            return Some(value);
        }
    }
    None
}

/// The bytes of a Windows file list (`CF_HDROP`): a `DROPFILES` header, then each full path as UTF-16 ended by a zero, and a
/// last zero. `wide` is always set: the paths are UTF-16.
pub fn drop_files_bytes(paths: &[PathBuf]) -> Vec<u8> {
    const HEADER: u32 = 20; // sizeof(DROPFILES): offset, point (2 x i32), fNC, fWide
    let mut out = Vec::new();
    out.extend_from_slice(&HEADER.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]); // pt.x, pt.y, fNC
    out.extend_from_slice(&1u32.to_le_bytes());
    for p in paths {
        // A path from `canonicalize` on Windows starts with \\?\ : the shell wants the plain form.
        let text = p.to_string_lossy();
        let plain = text.strip_prefix(r"\\?\").filter(|rest| !rest.starts_with("UNC\\")).unwrap_or(&text);
        for unit in plain.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A custom data pickle as Chromium writes it.
    fn pickle(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut put = |s: &str| {
            let units: Vec<u16> = s.encode_utf16().collect();
            payload.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in &units {
                payload.extend_from_slice(&u.to_le_bytes());
            }
            while payload.len() % 4 != 0 {
                payload.push(0);
            }
        };
        for (k, v) in entries {
            put(k);
            put(v);
        }
        let mut out = (payload.len() as u32).to_le_bytes().to_vec();
        out.extend(payload);
        out
    }

    fn file(name: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, b"x").unwrap();
        let text = path.to_string_lossy().to_string();
        (dir, text)
    }

    #[test]
    fn a_nonce_gives_its_files_once_and_nothing_after() {
        let (_d, a) = file("a.png");
        let nonce = register(&[a]).unwrap();
        let files = take(&nonce).expect("the files");
        assert_eq!(files.len(), 1);
        assert!(take(&nonce).is_none(), "used up by the first drag");
        assert!(take("not-a-nonce").is_none());
    }

    #[test]
    fn a_drag_with_a_path_that_fails_registers_nothing() {
        let (_d, a) = file("a.png");
        let missing = std::env::temp_dir().join("amx-no-such-file-xyz").to_string_lossy().to_string();
        assert!(register(&[a.clone(), missing]).is_err());
        assert!(register(&["relative/file.png".to_string()]).is_err());
        assert!(register(&[]).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(register(&[dir.path().to_string_lossy().to_string()]).unwrap_err().contains("not a file"));
        assert!(register(&[a]).is_ok());
    }

    #[test]
    fn a_file_in_a_vault_folder_is_never_dragged_out() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("Vault");
        std::fs::create_dir(&vault).unwrap();
        let secret = vault.join("key.bin");
        std::fs::write(&secret, b"x").unwrap();
        assert!(register(&[secret.to_string_lossy().to_string()]).unwrap_err().contains("vault"));
    }

    #[test]
    fn the_nonce_is_read_out_of_chromiums_custom_data() {
        let data = pickle(&[("text/plain", "hello"), (MARKER, "abc123"), ("x/y", "z")]);
        assert_eq!(marker_in(&data).as_deref(), Some("abc123"));
        assert_eq!(marker_in(&pickle(&[("text/plain", "hello")])), None);
        // Odd lengths pad to four bytes; a non-ASCII value reads whole.
        assert_eq!(marker_in(&pickle(&[("a", "b"), (MARKER, "né")])).as_deref(), Some("né"));
    }

    #[test]
    fn data_that_is_not_a_pickle_gives_no_nonce_and_never_panics() {
        for bad in [&b""[..], &b"abc"[..], &[255u8; 40][..], &[4, 0, 0, 0, 1, 0, 0, 0][..]] {
            assert_eq!(marker_in(bad), None);
        }
        let mut cut = pickle(&[(MARKER, "abcdef")]);
        cut.truncate(cut.len() - 6);
        assert_eq!(marker_in(&cut), None);
    }

    #[test]
    fn the_windows_file_list_has_its_header_the_paths_and_the_double_end() {
        let bytes = drop_files_bytes(&[PathBuf::from(r"\\?\C:\a\b.png"), PathBuf::from(r"C:\c.png")]);
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1, "wide characters");
        let units: Vec<u16> = bytes[20..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = String::from_utf16(&units).unwrap();
        assert_eq!(text, "C:\\a\\b.png\0C:\\c.png\0\0", "the long-path prefix is dropped");
    }
}
