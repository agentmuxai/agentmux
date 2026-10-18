// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Black out a browser pane's secret fields in a screenshot before an agent
//! sees it (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §5.1 item 4). The host finds the fields and their boxes in the page's CSS
//! pixels; this paints them over in the image, which is in device pixels.

use image::{DynamicImage, ImageFormat, Rgba};

/// The host's screenshot answer (`data.secret_boxes`, `data.css_width`)
/// applied to its decoded image: unchanged when the page has no secret
/// fields, otherwise blacked out and re-encoded. `Err` is the message to give
/// instead of the image.
pub(crate) fn redact_host_screenshot(
    host_resp: &serde_json::Value,
    bytes: Vec<u8>,
    encoded: String,
) -> Result<(Vec<u8>, String), String> {
    use base64::Engine as _;
    if host_resp.pointer("/data/secrets_unchecked").and_then(|v| v.as_bool()) == Some(true) {
        return Err("AgentMux couldn't check this page for password, card or tax-ID fields, so the screenshot \
                    isn't returned: take a snapshot instead, or try again once the page has loaded"
            .to_string());
    }
    let boxes: Vec<[f64; 4]> = host_resp
        .pointer("/data/secret_boxes")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if boxes.is_empty() {
        return Ok((bytes, encoded));
    }
    let css_width = host_resp.pointer("/data/css_width").and_then(|v| v.as_f64());
    let out = black_out(&bytes, &boxes, css_width)
        .map_err(|e| format!("couldn't black out the secret fields on this page, so the screenshot isn't returned: {e}"))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&out);
    Ok((out, encoded))
}

/// CSS pixels added around each box: a field's border and its focus ring.
const PAD: f64 = 3.0;

/// Paint every box black and return the image as PNG. `css_width` is the
/// page's viewport width, which scales the boxes to the image; without it
/// the boxes can't be placed, so it fails rather than guess.
pub(crate) fn black_out(image_bytes: &[u8], boxes: &[[f64; 4]], css_width: Option<f64>) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(image_bytes).map_err(|e| format!("read the image: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    let scale = match css_width {
        Some(cw) if cw > 0.0 => f64::from(w) / cw,
        _ => return Err("the page's width is unknown".to_string()),
    };
    for [x, y, bw, bh] in boxes {
        let to_px = |v: f64, limit: u32| -> u32 { (v * scale).round().clamp(0.0, f64::from(limit)) as u32 };
        let (x0, y0) = (to_px(x - PAD, w), to_px(y - PAD, h));
        let (x1, y1) = (to_px(x + bw + PAD, w), to_px(y + bh + PAD, h));
        for py in y0..y1 {
            for px in x0..x1 {
                img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
            }
        }
    }
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|e| format!("write the image: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn white_png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
        image::load_from_memory(png).unwrap().to_rgba8().get_pixel(x, y).0
    }

    #[test]
    fn a_box_is_painted_black_with_padding_and_the_rest_is_untouched() {
        let out = black_out(&white_png(200, 100), &[[50.0, 20.0, 40.0, 10.0]], Some(200.0)).unwrap();
        assert_eq!(pixel(&out, 70, 25), [0, 0, 0, 255], "inside the field");
        assert_eq!(pixel(&out, 48, 18), [0, 0, 0, 255], "inside the padding");
        assert_eq!(pixel(&out, 20, 25), [255, 255, 255, 255], "left of the field");
        assert_eq!(pixel(&out, 70, 60), [255, 255, 255, 255], "below the field");
    }

    #[test]
    fn boxes_are_scaled_from_css_pixels_to_the_images_device_pixels() {
        // A 2x screen: the page is 100 CSS pixels wide, the image 200.
        let out = black_out(&white_png(200, 100), &[[10.0, 10.0, 10.0, 10.0]], Some(100.0)).unwrap();
        assert_eq!(pixel(&out, 30, 30), [0, 0, 0, 255], "CSS (15, 15) is device (30, 30)");
        assert_eq!(pixel(&out, 60, 60), [255, 255, 255, 255], "past the box at 2x");
    }

    #[test]
    fn a_box_past_the_edge_is_clipped_not_an_error() {
        let out = black_out(&white_png(50, 50), &[[40.0, 40.0, 100.0, 100.0]], Some(50.0)).unwrap();
        assert_eq!(pixel(&out, 49, 49), [0, 0, 0, 255]);
    }

    #[test]
    fn a_host_answer_without_secret_fields_passes_through_untouched() {
        let png = white_png(4, 4);
        let (bytes, encoded) = redact_host_screenshot(&serde_json::json!({ "ok": true, "data": { "png_base64": "x" } }), png.clone(), "x".into()).unwrap();
        assert_eq!((bytes, encoded.as_str()), (png, "x"));
    }

    #[test]
    fn a_host_answer_with_secret_fields_is_redacted_or_refused() {
        let resp = serde_json::json!({ "data": { "secret_boxes": [[1.0, 1.0, 2.0, 2.0]], "css_width": 10.0 } });
        let (bytes, _) = redact_host_screenshot(&resp, white_png(10, 10), String::new()).unwrap();
        assert_eq!(pixel(&bytes, 2, 2), [0, 0, 0, 255]);
        let no_width = serde_json::json!({ "data": { "secret_boxes": [[1.0, 1.0, 2.0, 2.0]] } });
        assert!(redact_host_screenshot(&no_width, white_png(10, 10), String::new()).unwrap_err().contains("isn't returned"));
    }

    #[test]
    fn a_page_that_couldnt_be_checked_is_refused_even_with_no_boxes() {
        let resp = serde_json::json!({ "data": { "secrets_unchecked": true } });
        assert!(redact_host_screenshot(&resp, white_png(4, 4), String::new()).unwrap_err().contains("couldn't check this page"));
    }

    #[test]
    fn without_the_pages_width_it_refuses_rather_than_guess() {
        assert!(black_out(&white_png(10, 10), &[[1.0, 1.0, 2.0, 2.0]], None).is_err());
        assert!(black_out(&white_png(10, 10), &[[1.0, 1.0, 2.0, 2.0]], Some(0.0)).is_err());
    }
}
