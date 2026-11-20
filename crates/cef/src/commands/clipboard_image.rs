// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Put an image on the OS clipboard as an image (the Editor's, an agent reply's and the media viewer's Copy:
// frontend/app/element/media-copy.ts). The page hands over a PNG; this writes it the way each system's
// programs read an image from the clipboard. The browser's own clipboard call stays the way for a page
// without a host: in the embedded browser it needs the window focused and refuses what it doesn't like,
// so the host does it.

use serde_json::Value;

/// The most PNG bytes taken. The page limits itself to the same number (`IMAGE_COPY_MAX_BYTES`), so a
/// larger one is a page that did not check.
pub const IMAGE_MAX_BYTES: usize = 25 * 1024 * 1024;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// `{ png: <base64> }`: the image, on the clipboard as an image.
pub fn write_clipboard_image(args: &Value) -> Result<Value, String> {
    use base64::Engine as _;
    let encoded = args.get("png").and_then(|v| v.as_str()).ok_or_else(|| "missing 'png' argument".to_string())?;
    let png = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("the image is not valid base64: {e}"))?;
    check_png(&png)?;
    write_png(&png)?;
    Ok(Value::Null)
}

/// Whether `bytes` is a PNG within the size this takes.
fn check_png(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(format!("the image is larger than {} MB", IMAGE_MAX_BYTES / (1024 * 1024)));
    }
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err("the image is not a PNG".into());
    }
    Ok(())
}

/// A top-down 32-bit `BITMAPV5HEADER` DIB (`CF_DIBV5`) of straight-alpha RGBA pixels: BGRA in memory, with the
/// channel masks that say so. Windows derives `CF_DIB` and `CF_BITMAP` from it for programs that ask for those.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn dib_v5(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    const HEADER: u32 = 124;
    let mut out = Vec::with_capacity(HEADER as usize + rgba.len());
    out.extend_from_slice(&HEADER.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes()); // negative: rows run top-down
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    out.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // resolution and palette counts
    for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
        out.extend_from_slice(&mask.to_le_bytes()); // red, green, blue, alpha
    }
    out.extend_from_slice(&0x7352_4742u32.to_le_bytes()); // LCS_sRGB
    out.extend_from_slice(&[0u8; 64]); // endpoints, gamma, intent, profile fields, reserved
    debug_assert_eq!(out.len(), HEADER as usize);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

/// Windows: `CF_DIBV5` (what Paint, Office and most programs read) and the registered `PNG` format (what keeps
/// the alpha for the browsers and chat apps that read it), in one open of the clipboard.
#[cfg(target_os = "windows")]
fn write_png(png: &[u8]) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;

    /// `CF_DIBV5`.
    const CF_DIBV5: u32 = 17;

    unsafe fn block(bytes: &[u8]) -> *mut std::ffi::c_void {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if h.is_null() {
            return h;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        h
    }

    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| format!("can't read the image: {e}"))?
        .to_rgba8();
    let dib = dib_v5(image.width(), image.height(), image.as_raw());
    let png_format = {
        let name: Vec<u16> = "PNG".encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    };
    unsafe {
        // Another program (a clipboard manager) may hold the clipboard for a moment.
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            return Err("the clipboard is in use by another program; try again".into());
        }
        EmptyClipboard();
        // The system owns a block once SetClipboardData succeeds; if it fails the block is still ours to free.
        let h = block(&dib);
        if h.is_null() || SetClipboardData(CF_DIBV5, h).is_null() {
            if !h.is_null() {
                GlobalFree(h);
            }
            CloseClipboard();
            return Err("the system refused the image".into());
        }
        // The PNG is the extra; a program that reads only the DIB still gets the image.
        let h = block(png);
        if !h.is_null() && SetClipboardData(png_format, h).is_null() {
            GlobalFree(h);
        }
        CloseClipboard();
    }
    Ok(())
}

/// macOS: AppleScript sets the clipboard from the PNG file as `«class PNGf»` (the type its reader uses too).
#[cfg(target_os = "macos")]
fn write_png(png: &[u8]) -> Result<(), String> {
    let path = super::clipboard::write_clipboard_temp(png, "png")?;
    if path.contains(['"', '\\']) {
        return Err("the temporary file's name can't be used".into());
    }
    let script = format!("set the clipboard to (read (POSIX file \"{path}\") as «class PNGf»)");
    let out = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("osascript failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("the system refused the image: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Linux: `image/png` through wl-copy on Wayland, else xclip on X11.
#[cfg(target_os = "linux")]
fn write_png(png: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let wayland = std::env::var("WAYLAND_DISPLAY").is_ok_and(|v| !v.is_empty());
    let mut command = if wayland {
        let mut c = Command::new("wl-copy");
        c.args(["--type", "image/png"]);
        c
    } else {
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard", "-t", "image/png", "-i"]);
        c
    };
    let tool = if wayland { "wl-copy" } else { "xclip" };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("can't copy an image without {tool} (install it): {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(png).map_err(|e| format!("{tool} failed: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("{tool} failed: {e}"))?;
    if !status.success() {
        return Err(format!("{tool} refused the image"));
    }
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn write_png(_png: &[u8]) -> Result<(), String> {
    Err("copying an image is not supported on this system".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn only_a_png_within_the_limit_is_taken() {
        assert!(check_png(&png()).is_ok());
        assert_eq!(check_png(b"GIF89a"), Err("the image is not a PNG".into()));
        assert_eq!(check_png(&[]), Err("the image is not a PNG".into()));
        let mut big = png();
        big.resize(IMAGE_MAX_BYTES + 1, 0);
        assert!(check_png(&big).unwrap_err().contains("25 MB"));
    }

    #[test]
    fn a_missing_or_invalid_argument_is_refused_before_anything_is_written() {
        assert_eq!(write_clipboard_image(&serde_json::json!({})), Err("missing 'png' argument".into()));
        assert!(write_clipboard_image(&serde_json::json!({ "png": "***" })).unwrap_err().contains("base64"));
        // Valid base64 of something that is not a PNG.
        assert_eq!(write_clipboard_image(&serde_json::json!({ "png": "SGVsbG8=" })), Err("the image is not a PNG".into()));
    }

    #[test]
    fn the_dib_is_a_top_down_bgra_v5_bitmap() {
        // Two pixels: opaque red, then half-transparent blue (RGBA in, BGRA out).
        let dib = dib_v5(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        let u32_at = |o: usize| u32::from_le_bytes(dib[o..o + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 124, "the header's size");
        assert_eq!(u32_at(4), 2, "width");
        assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), -1, "negative height: top-down");
        assert_eq!(u32_at(16), 3, "BI_BITFIELDS");
        assert_eq!((u32_at(40), u32_at(44), u32_at(48), u32_at(52)), (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000));
        assert_eq!(&dib[124..], [0, 0, 255, 255, 255, 0, 0, 128]);
    }
}
