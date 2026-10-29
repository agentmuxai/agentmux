// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::io::Cursor;
use std::time::{Duration, Instant};

use base64::Engine as _;
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::json;

use super::*;

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
}

fn encoded(img: &RgbaImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    // JPEG has no alpha.
    let img = if format == ImageFormat::Jpeg { DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(img.clone()).to_rgb8()) } else { DynamicImage::ImageRgba8(img.clone()) };
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

/// The delays (ms) and sizes of every frame of a GIF.
fn read_gif(bytes: &[u8]) -> Vec<(u32, (u32, u32))> {
    let decoder = GifDecoder::new(Cursor::new(bytes.to_vec())).unwrap();
    decoder
        .into_frames()
        .map(|f| {
            let f = f.unwrap();
            let (n, d) = f.delay().numer_denom_ms();
            (n / d, f.buffer().dimensions())
        })
        .collect()
}

fn host_response(png: &[u8], extra: serde_json::Value) -> serde_json::Value {
    let mut data = json!({ "png_base64": base64::engine::general_purpose::STANDARD.encode(png) });
    for (k, v) in extra.as_object().unwrap() {
        data[k] = v.clone();
    }
    json!({ "ok": true, "data": data })
}

#[test]
fn each_frame_stays_for_as_long_as_it_was_on_screen_and_the_last_as_long_as_the_one_before() {
    let t0 = Instant::now();
    let mut b = GifBuilder::new();
    b.push(solid(40, 30, [255, 0, 0]), t0).unwrap();
    b.push(solid(40, 30, [0, 255, 0]), t0 + Duration::from_millis(100)).unwrap();
    b.push(solid(40, 30, [0, 0, 255]), t0 + Duration::from_millis(400)).unwrap();
    assert_eq!(b.frames(), 3);
    let (bytes, frames) = b.finish().unwrap();
    assert_eq!(frames, 3);
    assert_eq!(&bytes[..6], b"GIF89a");
    let got = read_gif(&bytes);
    assert_eq!(got.iter().map(|(ms, _)| *ms).collect::<Vec<_>>(), vec![100, 300, 300]);
}

#[test]
fn a_frame_is_never_shown_for_less_than_the_shortest_a_viewer_honours() {
    let t0 = Instant::now();
    let mut b = GifBuilder::new();
    b.push(solid(8, 8, [1, 2, 3]), t0).unwrap();
    b.push(solid(8, 8, [9, 9, 9]), t0 + Duration::from_millis(3)).unwrap();
    let (bytes, _) = b.finish().unwrap();
    assert!(read_gif(&bytes).iter().all(|(ms, _)| *ms >= 20));
}

#[test]
fn a_pane_resized_meanwhile_is_stretched_to_the_first_frames_size() {
    let t0 = Instant::now();
    let mut b = GifBuilder::new();
    b.push(solid(40, 30, [255, 0, 0]), t0).unwrap();
    b.push(solid(60, 30, [0, 255, 0]), t0 + Duration::from_millis(100)).unwrap();
    let (bytes, _) = b.finish().unwrap();
    let got = read_gif(&bytes);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|(_, size)| *size == (40, 30)), "{got:?}");
}

#[test]
fn no_frame_is_no_file() {
    assert!(GifBuilder::new().finish().is_err());
}

#[test]
fn a_wide_capture_is_scaled_to_the_frame_width_and_a_narrow_one_is_left_alone() {
    let wide = frame_from(&encoded(&solid(1600, 900, [10, 20, 30]), ImageFormat::Png)).unwrap();
    assert_eq!(wide.dimensions(), (FRAME_WIDTH, 450));
    let narrow = frame_from(&encoded(&solid(300, 200, [10, 20, 30]), ImageFormat::Png)).unwrap();
    assert_eq!(narrow.dimensions(), (300, 200));
    assert!(wide.pixels().chain(narrow.pixels()).all(|p| p.0[3] == 255));
}

#[test]
fn the_limits_are_held_whatever_is_asked() {
    assert_eq!(limits(None, None), (DEFAULT_FPS, MAX_SECONDS));
    assert_eq!(limits(Some(60), Some(600)), (MAX_FPS, MAX_SECONDS));
    assert_eq!(limits(Some(0), Some(0)), (1, 1));
    assert_eq!(limits(Some(8), Some(12)), (8, 12));
    assert_eq!((MAX_FPS, MAX_SECONDS), (10, 30));
}

#[test]
fn the_file_stays_under_the_size_one_attached_image_may_have() {
    assert!((MAX_BYTES as u64) < crate::backend::task_media::MAX_IMAGE_BYTES);
}

#[test]
fn a_frame_has_the_secret_fields_the_host_found_blacked_out() {
    let white = encoded(&solid(100, 50, [255, 255, 255]), ImageFormat::Png);
    let resp = host_response(&white, json!({ "secret_boxes": [[10.0, 10.0, 20.0, 10.0]], "css_width": 100.0 }));
    let frame = frame_of_response(&resp).unwrap();
    assert_eq!(frame.get_pixel(15, 15).0, [0, 0, 0, 255], "inside the field");
    assert_eq!(frame.get_pixel(80, 40).0, [255, 255, 255, 255], "outside it");
}

#[test]
fn a_jpeg_capture_is_read_too() {
    let jpeg = encoded(&solid(64, 48, [200, 200, 200]), ImageFormat::Jpeg);
    let frame = frame_of_response(&host_response(&jpeg, json!({}))).unwrap();
    assert_eq!(frame.dimensions(), (64, 48));
}

#[test]
fn a_frame_the_host_could_not_check_for_secrets_is_left_out() {
    let white = encoded(&solid(100, 50, [255, 255, 255]), ImageFormat::Png);
    let resp = host_response(&white, json!({ "secrets_unchecked": true }));
    let err = frame_of_response(&resp).unwrap_err();
    assert!(err.contains("couldn't check"), "{err}");
}

#[test]
fn a_host_refusal_is_not_a_frame() {
    let err = frame_of_response(&json!({ "ok": false, "error": "pane not found" })).unwrap_err();
    assert_eq!(err, "pane not found");
}

#[test]
fn the_pane_is_cut_out_of_the_whole_viewport_in_device_pixels() {
    // The page is 100 CSS pixels wide, shown at 2x: the image is 200 wide. The pane is at CSS (10, 10), 30 by 20.
    let pane = crop_to(&solid(200, 100, [1, 2, 3]), [10.0, 10.0, 30.0, 20.0], 100.0).unwrap();
    assert_eq!(pane.dimensions(), (60, 40));
}

#[test]
fn a_pane_that_cannot_be_cut_out_is_refused_not_guessed() {
    let img = solid(200, 100, [1, 2, 3]);
    assert!(crop_to(&img, [10.0, 10.0, 30.0, 20.0], 0.0).is_err(), "the page's width is unknown");
    assert!(crop_to(&img, [500.0, 10.0, 30.0, 20.0], 100.0).is_err(), "the pane is outside the capture");
}

#[test]
fn a_secret_field_is_blacked_out_in_the_pane_cut_from_a_whole_viewport_and_the_rest_is_gone() {
    let white = encoded(&solid(200, 100, [255, 255, 255]), ImageFormat::Png);
    // The boxes are from the pane's own origin, as the host measures them for a clipped capture.
    let resp = host_response(
        &white,
        json!({ "crop": [10.0, 10.0, 30.0, 20.0], "viewport_width": 100.0, "secret_boxes": [[5.0, 5.0, 10.0, 10.0]], "css_width": 30.0 }),
    );
    let frame = frame_of_response(&resp).unwrap();
    assert_eq!(frame.dimensions(), (60, 40), "only the pane");
    assert_eq!(frame.get_pixel(20, 20).0, [0, 0, 0, 255], "inside the field");
    assert_eq!(frame.get_pixel(50, 35).0, [255, 255, 255, 255], "outside it");
}

#[test]
fn a_whole_viewport_capture_the_host_could_not_scale_is_not_a_frame() {
    let white = encoded(&solid(200, 100, [255, 255, 255]), ImageFormat::Png);
    let resp = host_response(&white, json!({ "crop": [10.0, 10.0, 30.0, 20.0], "secrets_unchecked": true }));
    assert!(frame_of_response(&resp).is_err());
    let no_width = host_response(&white, json!({ "crop": [10.0, 10.0, 30.0, 20.0] }));
    assert!(frame_of_response(&no_width).unwrap_err().contains("width is unknown"));
}
