// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Clipboard commands — read/write the OS clipboard via Win32/macOS/Linux APIs.
// CEF's Chromium blocks navigator.clipboard.readText() without a permission
// policy header, so we route clipboard through the host process via IPC.

/// Read text from the OS clipboard.
pub fn read_clipboard() -> Result<serde_json::Value, String> {
    let text = read_clipboard_text()?;
    Ok(serde_json::json!(text))
}

/// Write text to the OS clipboard.
pub fn write_clipboard(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'text' argument".to_string())?;
    write_clipboard_text(text)?;
    Ok(serde_json::Value::Null)
}

#[cfg(target_os = "windows")]
fn read_clipboard_text() -> Result<String, String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("Failed to open clipboard".into());
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            CloseClipboard();
            return Ok(String::new());
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            CloseClipboard();
            return Err("Failed to lock clipboard data".into());
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);
        GlobalUnlock(handle);
        CloseClipboard();
        Ok(text)
    }
}

#[cfg(target_os = "windows")]
fn write_clipboard_text(text: &str) -> Result<(), String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let size = wide.len() * 2;

    unsafe {
        let hmem = GlobalAlloc(GMEM_MOVEABLE, size);
        if hmem.is_null() {
            return Err("Failed to allocate clipboard memory".into());
        }
        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            GlobalFree(hmem);
            return Err("Failed to lock clipboard memory".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(hmem);

        if OpenClipboard(std::ptr::null_mut()) == 0 {
            GlobalFree(hmem);
            return Err("Failed to open clipboard".into());
        }
        EmptyClipboard();
        // SetClipboardData returns NULL on failure — must be checked. On
        // success the system takes ownership of hmem; on failure it's still
        // ours to free (previously leaked here too).
        let result = SetClipboardData(CF_UNICODETEXT as u32, hmem);
        if result.is_null() {
            let err = std::io::Error::last_os_error();
            CloseClipboard();
            GlobalFree(hmem);
            return Err(format!("SetClipboardData failed: {}", err));
        }
        CloseClipboard();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn read_clipboard_text() -> Result<String, String> {
    use std::process::Command;
    let output = Command::new("pbpaste")
        .output()
        .map_err(|e| format!("pbpaste failed: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(target_os = "macos")]
fn write_clipboard_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pbcopy failed: {}", e))?;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(text.as_bytes())
        .map_err(|e| format!("pbcopy write failed: {}", e))?;
    child.wait().map_err(|e| format!("pbcopy wait failed: {}", e))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").map_or(false, |v| !v.is_empty())
}

#[cfg(target_os = "linux")]
fn read_clipboard_text() -> Result<String, String> {
    use std::process::Command;
    if is_wayland() {
        if let Ok(output) = Command::new("wl-paste").args(["--no-newline"]).output() {
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
        }
    }
    // X11 fallback: xclip, then xsel
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output()
        .or_else(|_| Command::new("xsel").args(["--clipboard", "--output"]).output())
        .map_err(|e| format!("clipboard read failed (install wl-paste, xclip, or xsel): {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(target_os = "linux")]
fn write_clipboard_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if is_wayland() {
        if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return Ok(());
                }
            }
        }
    }
    // X11 fallback: xclip, then xsel
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .spawn()
        .or_else(|_| {
            Command::new("xsel")
                .args(["--clipboard", "--input"])
                .stdin(Stdio::piped())
                .spawn()
        })
        .map_err(|e| format!("clipboard write failed (install wl-copy, xclip, or xsel): {}", e))?;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(text.as_bytes())
        .map_err(|e| format!("clipboard write failed: {}", e))?;
    child.wait().map_err(|e| format!("clipboard write failed: {}", e))?;
    Ok(())
}

// ── Attachments: text + files/images, for the agent composer's Paste ─────
//
// A right-click Paste has no `paste` event, so the renderer can't see the
// clipboard's files or image data. This returns the clipboard's text and a
// list of paths: the files themselves when files were copied (Explorer,
// Finder, a file manager), or a temp file holding the image when image data
// was copied (a screenshot). The renderer hands the paths to srv's
// `attachments.ingest`, which copies them into its store, so the bytes never
// pass through the renderer.
// docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.2.

/// Temp files older than this are removed on the next read; srv has copied
/// them long before.
const CLIPBOARD_TEMP_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

pub fn read_clipboard_attachments() -> Result<serde_json::Value, String> {
    let text = read_clipboard_text().unwrap_or_default();
    let paths = read_clipboard_paths().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "read_clipboard_attachments: no files or image read");
        Vec::new()
    });
    Ok(serde_json::json!({ "text": text, "paths": paths }))
}

/// Where image data read from the clipboard is written.
fn clipboard_temp_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("agentmux-clipboard")
}

/// Write `bytes` to a fresh temp file with extension `ext`, pruning old ones.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "linux")), allow(dead_code))]
fn write_clipboard_temp(bytes: &[u8], ext: &str) -> Result<String, String> {
    let dir = clipboard_temp_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let now = std::time::SystemTime::now();
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > CLIPBOARD_TEMP_MAX_AGE);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let path = dir.join(format!("clipboard-{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// A Windows DIB (`CF_DIB` / `CF_DIBV5`: BITMAPINFO header, optional masks
/// and color table, then pixels) as a `.bmp` file: the same bytes behind a
/// 14-byte BITMAPFILEHEADER whose pixel offset accounts for what sits
/// between the header and the pixels.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([dib[o], dib[o + 1], dib[o + 2], dib[o + 3]]);
    let u16_at = |o: usize| u16::from_le_bytes([dib[o], dib[o + 1]]);
    let header_size = u32_at(0) as usize;
    if header_size < 40 || header_size > dib.len() {
        return None;
    }
    let bit_count = u16_at(14) as u32;
    let compression = u32_at(16);
    let colors_used = u32_at(32);
    // BI_BITFIELDS (3) / BI_ALPHABITFIELDS (6) with a plain 40-byte header
    // store the masks right after it; larger headers hold them inside.
    let masks = match (header_size, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let palette_entries = if colors_used > 0 {
        colors_used
    } else if bit_count <= 8 {
        1 << bit_count
    } else {
        0
    };
    let offset = 14 + header_size + masks + palette_entries as usize * 4;
    let file_size = 14 + dib.len();
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

#[cfg(target_os = "windows")]
fn read_clipboard_paths() -> Result<Vec<String>, String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP};
    use windows_sys::Win32::UI::Shell::DragQueryFileW;

    /// Closes the clipboard on every return path.
    struct Opened;
    impl Drop for Opened {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }

    /// Copy an HGLOBAL clipboard handle's bytes out.
    unsafe fn global_bytes(handle: *mut core::ffi::c_void) -> Option<Vec<u8>> {
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() || size == 0 {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }

    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("Failed to open clipboard".into());
        }
        let _opened = Opened;

        // Files copied in Explorer.
        if IsClipboardFormatAvailable(CF_HDROP as u32) != 0 {
            let hdrop = GetClipboardData(CF_HDROP as u32);
            if !hdrop.is_null() {
                let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
                let mut paths = Vec::with_capacity(count as usize);
                for i in 0..count {
                    let len = DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0);
                    let mut buf = vec![0u16; len as usize + 1];
                    let got = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), len + 1);
                    if got > 0 {
                        paths.push(String::from_utf16_lossy(&buf[..got as usize]));
                    }
                }
                return Ok(paths);
            }
        }

        // Image data: browsers and the Snipping Tool also put a "PNG" format
        // up, which keeps transparency; fall back to the DIB.
        let png: Vec<u16> = "PNG".encode_utf16().chain(std::iter::once(0)).collect();
        let png_format = RegisterClipboardFormatW(png.as_ptr());
        if png_format != 0 && IsClipboardFormatAvailable(png_format) != 0 {
            if let Some(bytes) = global_bytes(GetClipboardData(png_format)) {
                return Ok(vec![write_clipboard_temp(&bytes, "png")?]);
            }
        }
        for format in [CF_DIBV5 as u32, CF_DIB as u32] {
            if IsClipboardFormatAvailable(format) == 0 {
                continue;
            }
            if let Some(bmp) = global_bytes(GetClipboardData(format)).and_then(|d| dib_to_bmp(&d)) {
                return Ok(vec![write_clipboard_temp(&bmp, "bmp")?]);
            }
        }
    }
    Ok(Vec::new())
}

/// macOS: a copied file via `«class furl»`, image data via `«class PNGf»`
/// (AppleScript prints it as `«data PNGf<hex>»`).
#[cfg(target_os = "macos")]
fn read_clipboard_paths() -> Result<Vec<String>, String> {
    use std::process::Command;
    let run = |script: &str| -> Option<String> {
        let out = Command::new("osascript").args(["-e", script]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if let Some(path) = run("POSIX path of (the clipboard as «class furl»)").filter(|p| !p.is_empty()) {
        return Ok(vec![path]);
    }
    if let Some(data) = run("the clipboard as «class PNGf»") {
        if let Some(hex) = data.strip_prefix("«data PNGf").and_then(|s| s.strip_suffix('»')) {
            let bytes: Option<Vec<u8>> = (0..hex.len())
                .step_by(2)
                .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
                .collect();
            if let Some(bytes) = bytes.filter(|b| !b.is_empty()) {
                return Ok(vec![write_clipboard_temp(&bytes, "png")?]);
            }
        }
    }
    Ok(Vec::new())
}

/// Linux: `text/uri-list` for copied files, `image/png` for image data, via
/// wl-paste on Wayland and xclip on X11.
#[cfg(target_os = "linux")]
fn read_clipboard_paths() -> Result<Vec<String>, String> {
    use std::process::Command;
    let read = |mime: &str| -> Option<Vec<u8>> {
        let out = if is_wayland() {
            Command::new("wl-paste").args(["--no-newline", "--type", mime]).output().ok()?
        } else {
            Command::new("xclip").args(["-selection", "clipboard", "-t", mime, "-o"]).output().ok()?
        };
        (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
    };
    let types = if is_wayland() {
        Command::new("wl-paste").arg("--list-types").output().ok()
    } else {
        Command::new("xclip").args(["-selection", "clipboard", "-t", "TARGETS", "-o"]).output().ok()
    }
    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    .unwrap_or_default();
    if types.lines().any(|t| t.trim() == "text/uri-list") {
        if let Some(list) = read("text/uri-list") {
            let paths: Vec<String> = String::from_utf8_lossy(&list)
                .lines()
                .filter_map(|l| l.trim().strip_prefix("file://").map(percent_decode))
                .collect();
            if !paths.is_empty() {
                return Ok(paths);
            }
        }
    }
    if types.lines().any(|t| t.trim() == "image/png") {
        if let Some(bytes) = read("image/png") {
            return Ok(vec![write_clipboard_temp(&bytes, "png")?]);
        }
    }
    Ok(Vec::new())
}

#[cfg(target_os = "linux")]
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod attachment_tests {
    use super::dib_to_bmp;

    fn dib(header: u32, bit_count: u16, compression: u32, colors_used: u32, pixels: usize) -> Vec<u8> {
        let mut d = vec![0u8; header as usize];
        d[0..4].copy_from_slice(&header.to_le_bytes());
        d[4..8].copy_from_slice(&2i32.to_le_bytes()); // width
        d[8..12].copy_from_slice(&2i32.to_le_bytes()); // height
        d[12..14].copy_from_slice(&1u16.to_le_bytes()); // planes
        d[14..16].copy_from_slice(&bit_count.to_le_bytes());
        d[16..20].copy_from_slice(&compression.to_le_bytes());
        d[32..36].copy_from_slice(&colors_used.to_le_bytes());
        d.extend(std::iter::repeat_n(0u8, pixels));
        d
    }

    fn offset(bmp: &[u8]) -> u32 {
        u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]])
    }

    #[test]
    fn plain_24_bit_dib_gets_a_file_header() {
        let bmp = dib_to_bmp(&dib(40, 24, 0, 0, 16)).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes([bmp[2], bmp[3], bmp[4], bmp[5]]) as usize, bmp.len());
        assert_eq!(offset(&bmp), 14 + 40);
    }

    #[test]
    fn bitfield_masks_and_palettes_move_the_pixel_offset() {
        assert_eq!(offset(&dib_to_bmp(&dib(40, 32, 3, 0, 16)).unwrap()), 14 + 40 + 12);
        // A V5 header holds its masks itself.
        assert_eq!(offset(&dib_to_bmp(&dib(124, 32, 3, 0, 16)).unwrap()), 14 + 124);
        // 8-bit: a 256-entry palette unless biClrUsed says otherwise.
        assert_eq!(offset(&dib_to_bmp(&dib(40, 8, 0, 0, 4)).unwrap()), 14 + 40 + 1024);
        assert_eq!(offset(&dib_to_bmp(&dib(40, 8, 0, 16, 4)).unwrap()), 14 + 40 + 64);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(dib_to_bmp(&[1, 2, 3]).is_none());
        assert!(dib_to_bmp(&dib(40, 24, 0, 0, 0)[..20]).is_none());
    }
}
