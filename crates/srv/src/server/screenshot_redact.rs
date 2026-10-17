// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Black out a browser pane's secret fields in a screenshot before an agent
//! sees it (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §5.1 item 4). The host finds the fields and their boxes in the page's CSS
//! pixels; this paints them over in the image, which is in device pixels.

use image::{DynamicImage, ImageFormat, Rgba};

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
    fn without_the_pages_width_it_refuses_rather_than_guess() {
        assert!(black_out(&white_png(10, 10), &[[1.0, 1.0, 2.0, 2.0]], None).is_err());
        assert!(black_out(&white_png(10, 10), &[[1.0, 1.0, 2.0, 2.0]], Some(0.0)).is_err());
    }
}
