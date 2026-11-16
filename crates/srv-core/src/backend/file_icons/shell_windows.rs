// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Windows shell's icon for a file type, as a PNG. Runs only in the helper
//! process ([`super::child`]), on its one COM apartment thread. The name asked
//! about never exists: with `SHGFI_USEFILEATTRIBUTES` the shell answers from the
//! registry's file associations alone, so no disk is touched and a file on an SSH
//! host gets the icon this computer would show for that type (spec §2, §7).

use std::io::Cursor;

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP,
    HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

use super::request::{native_list, IconRequest, Kind};

/// Why no picture was made, as the helper reports it.
#[derive(Debug)]
pub enum ShellError {
    /// The shell has no icon for the type.
    NoIcon,
    /// Something in the calls failed (the text is for the log).
    Failed(&'static str),
}

/// Join this thread to a single-threaded COM apartment, which the shell expects.
pub fn init() -> Result<(), &'static str> {
    // SAFETY: plain COM initialisation of the calling thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok().map_err(|_| "com-init")
}

/// A bitmap handle deleted when it goes out of scope.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle came from GetIconInfo and is deleted once, here.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

/// The icon's pixels as straight-alpha RGBA, with its size.
fn hicon_pixels(icon: HICON) -> Result<(Vec<u8>, u32, u32), &'static str> {
    let mut info = ICONINFO::default();
    // SAFETY: `icon` is a live handle; `info` is written by the call.
    unsafe { GetIconInfo(icon, &mut info) }.map_err(|_| "icon-info")?;
    let (color, mask) = (Bitmap(info.hbmColor), Bitmap(info.hbmMask));
    if color.0.is_invalid() {
        return Err("monochrome icon");
    }
    let mut bm = BITMAP::default();
    // SAFETY: `bm` is a BITMAP of the size passed.
    let got = unsafe { GetObjectW(HGDIOBJ(color.0 .0), size_of::<BITMAP>() as i32, Some((&raw mut bm).cast())) };
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 || bm.bmWidth > 1024 || bm.bmHeight > 1024 {
        return Err("bitmap size");
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    // SAFETY: the screen DC is released below; GetDIBits fills a buffer of exactly w*h*4 bytes.
    let (color_px, mask_bgra) = unsafe {
        let dc = GetDC(None);
        let read = |hbm: &Bitmap| -> Option<Vec<u8>> {
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(dc, hbm.0, 0, h as u32, Some(buf.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
            (lines != 0).then_some(buf)
        };
        let out = (read(&color), read(&mask));
        ReleaseDC(None, dc);
        out
    };
    let mut bgra = color_px.ok_or("pixels")?;
    // An icon without an alpha channel says which pixels are see-through in its mask (white = clear).
    if bgra.chunks_exact(4).all(|p| p[3] == 0) {
        let mask = mask_bgra.ok_or("mask")?;
        for (p, m) in bgra.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            p[3] = if m[0] == 0 && m[1] == 0 && m[2] == 0 { 255 } else { 0 };
        }
    }
    for p in bgra.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    Ok((bgra, w as u32, h as u32))
}

/// The PNG of the icon the shell shows for `req`'s type, `req.px` square.
pub fn render(req: &IconRequest) -> Result<Vec<u8>, ShellError> {
    let name: Vec<u16> = req.shell_name().encode_utf16().chain(std::iter::once(0)).collect();
    let attributes = if req.kind == Kind::Dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let mut info = SHFILEINFOW::default();
    // SAFETY: `name` is NUL-terminated and outlives the call; `info` is the size passed.
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            attributes,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES,
        )
    };
    if ok == 0 {
        return Err(ShellError::Failed("file-info"));
    }
    let (list_number, _) = native_list(req.px);
    // SAFETY: the shell returns the system image list for that size.
    let list: IImageList = unsafe { SHGetImageList(list_number) }.map_err(|_| ShellError::Failed("image-list"))?;
    // SAFETY: the index came from the same list.
    let icon = unsafe { list.GetIcon(info.iIcon, ILD_TRANSPARENT.0) }.map_err(|_| ShellError::Failed("get-icon"))?;
    let pixels = hicon_pixels(icon);
    // SAFETY: the icon was created by GetIcon and is destroyed once.
    unsafe {
        let _ = DestroyIcon(icon);
    }
    let (rgba, w, h) = pixels.map_err(ShellError::Failed)?;
    let mut img = image::RgbaImage::from_raw(w, h, rgba).ok_or(ShellError::Failed("pixels"))?;
    if (w, h) != (req.px, req.px) {
        img = image::imageops::resize(&img, req.px, req.px, image::imageops::FilterType::Lanczos3);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| ShellError::Failed("png"))?;
    Ok(png)
}

/// The real helper asks the shell from one thread. Tests run in parallel threads, and the shell's
/// first use in a process doesn't take concurrent callers, so the tests that ask it take turns.
#[cfg(test)]
pub static SHELL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: Kind, ext: &str, px: u32) -> IconRequest {
        IconRequest { kind, ext: ext.to_string(), px }
    }

    fn decode(png: &[u8]) -> image::RgbaImage {
        image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("a PNG").to_rgba8()
    }

    #[test]
    fn a_type_icon_comes_back_as_a_png_of_the_size_asked_with_something_drawn() {
        let _turn = SHELL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        init().unwrap();
        for ext in ["txt", "rs", "exe", "zip"] {
            for px in [16, 20, 24, 32, 48, 64] {
                let img = decode(&render(&req(Kind::File, ext, px)).unwrap_or_else(|_| panic!("{ext} {px}")));
                assert_eq!(img.dimensions(), (px, px), "{ext} at {px}");
                assert!(img.pixels().any(|p| p[3] > 0), "{ext} at {px} is not blank");
                assert!(img.pixels().any(|p| p[3] == 0), "{ext} at {px} has a see-through corner, so alpha survived");
            }
        }
    }

    #[test]
    fn a_folder_a_file_without_an_extension_and_an_unknown_type_all_get_an_icon() {
        let _turn = SHELL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        init().unwrap();
        for r in [req(Kind::Dir, "", 16), req(Kind::File, "", 16), req(Kind::File, "nope_zz", 16)] {
            let img = decode(&render(&r).unwrap_or_else(|_| panic!("{r:?}")));
            assert!(img.pixels().any(|p| p[3] > 0), "{r:?}");
        }
    }

    #[test]
    fn a_folder_and_a_file_do_not_look_alike() {
        let _turn = SHELL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        init().unwrap();
        let dir = decode(&render(&req(Kind::Dir, "", 32)).unwrap());
        let file = decode(&render(&req(Kind::File, "", 32)).unwrap());
        assert_ne!(dir.as_raw(), file.as_raw());
    }
}
