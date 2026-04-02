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
        SetClipboardData(CF_UNICODETEXT as u32, hmem);
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
fn read_clipboard_text() -> Result<String, String> {
    use std::process::Command;
    // Try Wayland first (wl-paste), then X11 (xclip, xsel)
    let output = Command::new("wl-paste")
        .args(["--no-newline"])
        .output()
        .or_else(|_| {
            Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .output()
        })
        .or_else(|_| Command::new("xsel").args(["--clipboard", "--output"]).output())
        .map_err(|e| format!("clipboard read failed (install wl-paste, xclip, or xsel): {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(target_os = "linux")]
fn write_clipboard_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // Try Wayland first (wl-copy), then X11 (xclip, xsel)
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .or_else(|_| {
            Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .spawn()
        })
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
    child.wait().map_err(|e| format!("clipboard wait failed: {}", e))?;
    Ok(())
}
