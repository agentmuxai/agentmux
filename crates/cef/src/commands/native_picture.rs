// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A native copy of a window's pixels, the spike of the host-armed tear-off picture (#48707): how long does the
// host take to copy the window it shows, and are the pixels right? Compared with the CDP screenshot the tear-off
// takes today (50 to 130 ms in the host, with the page's frames held while it runs).
//
// Nothing here keeps a picture: the copy is measured and dropped inside the call, never written to disk or a log.
// The log line carries times, sizes and two percentages (how much of the picture is not black, and how much of it
// matches the CDP screenshot of the same moment, on a coarse grid), nothing of the content.

/// Cells per side of the grid two pictures are compared on.
#[cfg(any(target_os = "windows", test))]
const GRID: usize = 48;
/// How far two cells' mean channels may differ and still be the same picture (JPEG and colour handling).
#[cfg(any(target_os = "windows", test))]
const CELL_TOLERANCE: i32 = 40;
/// A cell whose mean is darker than this on every channel is black (nothing was copied there).
#[cfg(any(target_os = "windows", test))]
const BLACK_BELOW: i32 = 8;

/// The mean colour (r, g, b) of each cell of a `GRID` x `GRID` grid over a picture: `pixel(x, y)` reads a pixel.
/// A handful of pixels per cell is enough for a coarse comparison.
#[cfg(any(target_os = "windows", test))]
fn grid_means(
    width: usize,
    height: usize,
    pixel: impl Fn(usize, usize) -> (i32, i32, i32),
) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::with_capacity(GRID * GRID);
    for gy in 0..GRID {
        for gx in 0..GRID {
            let (x0, x1) = (
                gx * width / GRID,
                ((gx + 1) * width / GRID).max(gx * width / GRID + 1),
            );
            let (y0, y1) = (
                gy * height / GRID,
                ((gy + 1) * height / GRID).max(gy * height / GRID + 1),
            );
            let mut sum = (0, 0, 0);
            let mut n = 0;
            for (x, y) in [
                (x0, y0),
                (x1 - 1, y0),
                (x0, y1 - 1),
                (x1 - 1, y1 - 1),
                ((x0 + x1) / 2, (y0 + y1) / 2),
            ] {
                let p = pixel(x.min(width - 1), y.min(height - 1));
                sum = (sum.0 + p.0, sum.1 + p.1, sum.2 + p.2);
                n += 1;
            }
            out.push((sum.0 / n, sum.1 / n, sum.2 / n));
        }
    }
    out
}

/// Percent of cells that are not black, and percent of cells where `a` and `b` agree within the tolerance.
#[cfg(any(target_os = "windows", test))]
fn compare_grids(a: &[(i32, i32, i32)], b: &[(i32, i32, i32)]) -> (u32, u32) {
    let n = a.len().min(b.len()).max(1);
    let nonblank = a
        .iter()
        .filter(|c| c.0 >= BLACK_BELOW || c.1 >= BLACK_BELOW || c.2 >= BLACK_BELOW)
        .count();
    let same = a
        .iter()
        .zip(b)
        .filter(|(p, q)| {
            (p.0 - q.0).abs() <= CELL_TOLERANCE
                && (p.1 - q.1).abs() <= CELL_TOLERANCE
                && (p.2 - q.2).abs() <= CELL_TOLERANCE
        })
        .count();
    ((nonblank * 100 / n) as u32, (same * 100 / n) as u32)
}

#[cfg(target_os = "windows")]
mod win {
    use super::{compare_grids, grid_means};
    use std::sync::Arc;

    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::Storage::Xps::{PrintWindow, PW_CLIENTONLY};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    use crate::state::AppState;

    /// `PW_RENDERFULLCONTENT`: windows-sys does not name it. It makes `PrintWindow` take the composited page, not just what GDI draws.
    const PW_RENDERFULLCONTENT: u32 = 2;

    /// A copy of a window's client area, top-down BGRA.
    struct Grab {
        width: usize,
        height: usize,
        bgra: Vec<u8>,
    }

    /// Copy the client area of `hwnd` as the window draws it, whatever covers it: `PrintWindow` asks the window for
    /// its own content (the full-content flag takes the composited, GPU-drawn page), not the screen under it.
    fn grab(hwnd: *mut std::ffi::c_void) -> Result<Grab, String> {
        unsafe {
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClientRect(hwnd, &mut rc) == 0 || rc.right <= rc.left || rc.bottom <= rc.top {
                return Err("the window has no client area (minimised?)".to_string());
            }
            let (w, h) = ((rc.right - rc.left) as usize, (rc.bottom - rc.top) as usize);
            let window_dc = GetDC(hwnd);
            if window_dc.is_null() {
                return Err("no device context for the window".to_string());
            }
            let dc = CreateCompatibleDC(window_dc);
            let bitmap = CreateCompatibleBitmap(window_dc, w as i32, h as i32);
            let old = SelectObject(dc, bitmap);
            let printed = PrintWindow(hwnd, dc, PW_CLIENTONLY | PW_RENDERFULLCONTENT);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let mut bgra = vec![0u8; w * h * 4];
            let lines = if printed != 0 {
                GetDIBits(
                    dc,
                    bitmap,
                    0,
                    h as u32,
                    bgra.as_mut_ptr().cast(),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(hwnd, window_dc);
            if printed == 0 {
                return Err("PrintWindow failed".to_string());
            }
            if lines == 0 {
                return Err("GetDIBits copied no lines".to_string());
            }
            Ok(Grab {
                width: w,
                height: h,
                bgra,
            })
        }
    }

    /// `debug_native_window_picture`: copy the window `label` natively, log how long it took, and drop it. With
    /// `compare`, also take the CDP screenshot of the same moment and log how much of the two agree.
    pub async fn measure(
        state: &Arc<AppState>,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use base64::Engine;
        let label = args
            .get("label")
            .and_then(|v| v.as_str())
            .ok_or("debug_native_window_picture: label is required")?;
        let compare = args
            .get("compare")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let hwnd = unsafe { crate::commands::window::resolve_window_hwnd_strict(state, label) }
            .ok_or("debug_native_window_picture: no such window")? as usize;
        let started = std::time::Instant::now();
        let copied = tokio::task::spawn_blocking(move || grab(hwnd as *mut std::ffi::c_void))
            .await
            .map_err(|e| format!("debug_native_window_picture join error: {}", e))?;
        let native_ms = started.elapsed().as_millis() as u64;
        let grabbed = match copied {
            Ok(g) => g,
            Err(e) => {
                tracing::info!(target: "dnd:tearoff", native_ms, error = %e, "[tearoff-perf] native window picture failed");
                return Ok(serde_json::json!({ "ok": false }));
            }
        };
        let (mut cdp_ms, mut cdp_size, mut similar_pct) = (0u64, (0u32, 0u32), None);
        let native_cells = grid_means(grabbed.width, grabbed.height, |x, y| {
            let i = (y * grabbed.width + x) * 4;
            (
                grabbed.bgra[i + 2] as i32,
                grabbed.bgra[i + 1] as i32,
                grabbed.bgra[i] as i32,
            )
        });
        let mut nonblank_pct = compare_grids(&native_cells, &native_cells).0;
        if compare {
            let cdp_started = std::time::Instant::now();
            let jpeg =
                crate::browser_api::routes::capture_window_viewport(state, label, 80).await?;
            cdp_ms = cdp_started.elapsed().as_millis() as u64;
            let cells = tokio::task::spawn_blocking(
                move || -> Result<((u32, u32), Vec<(i32, i32, i32)>), String> {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(jpeg)
                        .map_err(|e| e.to_string())?;
                    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                        .map_err(|e| e.to_string())?
                        .into_rgb8();
                    let (w, h) = (img.width() as usize, img.height() as usize);
                    let cells = grid_means(w, h, |x, y| {
                        let p = img.get_pixel(x as u32, y as u32);
                        (p[0] as i32, p[1] as i32, p[2] as i32)
                    });
                    Ok(((img.width(), img.height()), cells))
                },
            )
            .await
            .map_err(|e| format!("debug_native_window_picture join error: {}", e))??;
            cdp_size = cells.0;
            let (nb, same) = compare_grids(&native_cells, &cells.1);
            nonblank_pct = nb;
            similar_pct = Some(same);
        }
        tracing::info!(
            target: "dnd:tearoff",
            native_ms, cdp_ms, width = grabbed.width, height = grabbed.height, cdp_width = cdp_size.0, cdp_height = cdp_size.1,
            nonblank_pct, similar_pct = similar_pct.map_or(-1, |s| s as i64), compare,
            "[tearoff-perf] native window picture"
        );
        Ok(serde_json::json!({ "ok": true }))
    }
}

#[cfg(target_os = "windows")]
pub use win::measure;

/// Only Windows has the native copy this measures.
#[cfg(not(target_os = "windows"))]
pub async fn measure(
    _state: &std::sync::Arc<crate::state::AppState>,
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Err("debug_native_window_picture is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::{compare_grids, grid_means, GRID};

    #[test]
    fn a_flat_picture_is_all_one_colour_and_matches_itself_at_another_size() {
        let a = grid_means(400, 300, |_, _| (200, 100, 50));
        let b = grid_means(200, 150, |_, _| (205, 98, 55));
        assert_eq!(a.len(), GRID * GRID);
        assert_eq!(compare_grids(&a, &b), (100, 100));
    }

    #[test]
    fn a_black_copy_is_blank_and_does_not_match_a_page() {
        let black = grid_means(300, 300, |_, _| (0, 0, 0));
        let page = grid_means(300, 300, |_, _| (240, 240, 240));
        assert_eq!(compare_grids(&black, &page), (0, 0));
    }

    #[test]
    fn half_a_picture_missing_matches_about_half() {
        let whole = grid_means(480, 480, |_, _| (120, 120, 120));
        let half = grid_means(
            480,
            480,
            |_, y| if y < 240 { (120, 120, 120) } else { (0, 0, 0) },
        );
        let (nonblank, same) = compare_grids(&half, &whole);
        assert!(
            (45..=55).contains(&nonblank) && (45..=55).contains(&same),
            "{nonblank} {same}"
        );
    }
}
