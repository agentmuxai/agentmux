// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A pane's picture for a tear-off, cut out of a picture of its window here in the host. The page used to do it:
// it took the window's whole picture over the host channel, decoded it, drew the pane into a canvas and encoded
// that, all on the page's main thread, which is the thread the drag needs for its first frames.

use std::sync::Arc;

use crate::state::AppState;

/// The quality the cut picture is encoded at, as the page encoded it (0.8).
#[cfg(target_os = "windows")]
const CUT_QUALITY: u8 = 80;

/// The pane's rect of the page (left, top, width, height, CSS px) cut out of `jpeg`, a picture of the page's
/// whole viewport that is `viewport_width` CSS px wide, as a JPEG. None when nothing of the pane is in it.
#[cfg(target_os = "windows")]
pub(crate) fn cut_pane(
    jpeg: &[u8],
    rect: (f64, f64, f64, f64),
    viewport_width: f64,
) -> Result<Option<Vec<u8>>, String> {
    use crate::commands::live_follow_picture::crop_rect;
    let window = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .map_err(|e| format!("picture does not decode: {}", e))?
        .into_rgb8();
    let Some((x, y, w, h)) = crop_rect(rect, viewport_width, (window.width(), window.height()))
    else {
        return Ok(None);
    };
    let pane = image::imageops::crop_imm(&window, x, y, w, h).to_image();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, CUT_QUALITY)
        .encode(pane.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("pane picture does not encode: {}", e))?;
    Ok(Some(out))
}

/// How many window pictures are kept waiting for their cut at once, and for how long. One drag takes one picture
/// per pane it shows (the press's, and the landing places'), and each is cut within moments of being taken.
#[cfg(target_os = "windows")]
const KEPT_MAX: usize = 4;
#[cfg(target_os = "windows")]
const KEPT_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(15);

/// A window's picture the host has taken and keeps until the page asks for the pane's cut of it.
#[cfg(target_os = "windows")]
struct Kept {
    id: u64,
    at: std::time::Instant,
    /// The window's whole viewport, a base64 JPEG as the capture answered it.
    jpeg_base64: String,
}

#[cfg(target_os = "windows")]
static KEPT: std::sync::LazyLock<std::sync::Mutex<Vec<Kept>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
#[cfg(target_os = "windows")]
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Keep a picture; the oldest go when there are too many or they are too old. Answers its id.
#[cfg(target_os = "windows")]
fn keep(jpeg_base64: String) -> u64 {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|k| k.at.elapsed() < KEPT_MAX_AGE);
    kept.push(Kept {
        id,
        at: std::time::Instant::now(),
        jpeg_base64,
    });
    while kept.len() > KEPT_MAX {
        kept.remove(0);
    }
    id
}

/// The picture kept under `id`, handed over once.
#[cfg(target_os = "windows")]
fn take_kept(id: u64) -> Option<String> {
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let at = kept.iter().position(|k| k.id == id)?;
    let k = kept.remove(at);
    (k.at.elapsed() < KEPT_MAX_AGE).then_some(k.jpeg_base64)
}

#[cfg(target_os = "windows")]
fn num(args: &serde_json::Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("cut_pane_picture: {} is required", key))
}

/// `capture_pane_picture`: take the window's picture and keep it in the host, for the cuts of the page's tear-off
/// snapshots. `label` (the window) and `quality` (of the picture). Answers `{ pictureId }` as soon as the window
/// has been pictured: the drag's drawing is held only until then, not for the cut. The window is pictured whole, as
/// for any tear-off picture (a clipped capture flickers the window).
#[cfg(target_os = "windows")]
pub async fn capture(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("capture_pane_picture: label is required")?;
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(80) as u8;
    let started = std::time::Instant::now();
    let whole = crate::browser_api::routes::capture_window_viewport(state, label, quality).await?;
    tracing::info!(
        target: "dnd:tearoff",
        pictured_ms = started.elapsed().as_millis() as u64,
        "[tearoff-perf] the window was pictured and kept in the host"
    );
    Ok(serde_json::json!({ "pictureId": keep(whole) }))
}

/// `cut_pane_picture`: the pane cut out of the picture kept under `pictureId`. `rect` (`left`, `top`, `width`,
/// `height`: the pane, CSS px) and `viewportWidth` (CSS px). Answers `{ jpeg_base64 }`, null when the pane is not in
/// the picture; an error when the picture is gone (cut already, or too old).
#[cfg(target_os = "windows")]
pub async fn cut(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    use base64::Engine;
    let id = args
        .get("pictureId")
        .and_then(|v| v.as_u64())
        .ok_or("cut_pane_picture: pictureId is required")?;
    let rect = args
        .get("rect")
        .ok_or("cut_pane_picture: rect is required")?;
    let rect = (
        num(rect, "left")?,
        num(rect, "top")?,
        num(rect, "width")?,
        num(rect, "height")?,
    );
    let viewport_width = num(args, "viewportWidth")?;
    let whole = take_kept(id).ok_or("cut_pane_picture: the window picture is gone")?;
    let started = std::time::Instant::now();
    let cut = tokio::task::spawn_blocking(move || {
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(whole)
            .map_err(|e| format!("picture is not base64: {}", e))?;
        cut_pane(&jpeg, rect, viewport_width)
    })
    .await
    .map_err(|e| format!("cut_pane_picture join error: {}", e))??;
    tracing::info!(
        target: "dnd:tearoff",
        cut_ms = started.elapsed().as_millis() as u64, found = cut.is_some(),
        "[tearoff-perf] the pane was cut out of the window picture in the host"
    );
    let b64 = cut.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
    Ok(serde_json::json!({ "jpeg_base64": b64 }))
}

/// Only Windows pictures a tear-off pane this way: the other platforms take no picture of it.
#[cfg(not(target_os = "windows"))]
pub async fn capture(
    _state: &Arc<AppState>,
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Err("capture_pane_picture is only available on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn cut(_args: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("cut_pane_picture is only available on Windows".to_string())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{cut_pane, keep, take_kept};

    /// A picture `w` x `h` whose left half is red and right half blue, as a JPEG.
    fn halves(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            }
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    fn decoded(jpeg: &[u8]) -> image::RgbImage {
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8()
    }

    #[test]
    fn the_pane_is_cut_out_at_the_pictures_own_scale() {
        // 400 x 200 picture of a viewport 200 CSS px wide (a scale of 2): the pane at CSS (100, 0) 100 x 100 is
        // the right half's top, 200 x 200 pixels, all blue.
        let cut = cut_pane(&halves(400, 200), (100.0, 0.0, 100.0, 100.0), 200.0)
            .unwrap()
            .unwrap();
        let pane = decoded(&cut);
        assert_eq!((pane.width(), pane.height()), (200, 200));
        let p = pane.get_pixel(100, 100);
        assert!(p[2] > 150 && p[0] < 80, "expected blue, got {:?}", p);
    }

    #[test]
    fn a_pane_past_the_edge_is_cut_to_what_is_inside_and_one_outside_gives_none() {
        let whole = halves(200, 100);
        let edge = decoded(
            &cut_pane(&whole, (150.0, 50.0, 100.0, 100.0), 200.0)
                .unwrap()
                .unwrap(),
        );
        assert_eq!((edge.width(), edge.height()), (50, 50));
        assert!(cut_pane(&whole, (300.0, 0.0, 50.0, 50.0), 200.0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_kept_picture_is_handed_over_once_and_the_oldest_go_when_there_are_too_many() {
        let first = keep("one".to_string());
        assert_eq!(take_kept(first).as_deref(), Some("one"));
        assert!(take_kept(first).is_none());
        let ids: Vec<u64> = (0..super::KEPT_MAX + 1)
            .map(|i| keep(format!("p{}", i)))
            .collect();
        assert!(take_kept(ids[0]).is_none());
        assert_eq!(take_kept(ids[1]).as_deref(), Some("p1"));
    }

    #[test]
    fn a_picture_that_does_not_decode_is_an_error() {
        assert!(cut_pane(b"not a jpeg", (0.0, 0.0, 10.0, 10.0), 100.0).is_err());
    }
}
