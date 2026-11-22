// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A frame of the virtual screen as the pane wants it: the same kind byte and JPEG the streamed browser sends
//! (`pane.rs`), made only when the screen changed since the last one that was sent.

use std::io::Cursor;

/// The first byte of a frame message (the pane's `FRAME`).
pub const FRAME: u8 = 1;
/// The same quality as the streamed browser's screencast.
const JPEG_QUALITY: u8 = 70;

/// Remembers the last screen that was sent, to say whether the next one is worth sending.
#[derive(Default)]
pub struct Changes {
    last: Vec<u8>,
}

impl Changes {
    /// True when `pixels` differs from the last one handed in (or is the first). Remembers it.
    pub fn changed(&mut self, pixels: &[u8]) -> bool {
        if self.last == pixels {
            return false;
        }
        self.last.clear();
        self.last.extend_from_slice(pixels);
        true
    }
}

/// The screen's pixels as the X server hands them (blue, green, red, one unused byte per pixel) as one frame message:
/// the kind byte, then a JPEG.
pub fn frame_message(bgrx: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let pixels = (w as usize) * (h as usize);
    if bgrx.len() < pixels * 4 {
        return Err(format!("the screen came back with {} bytes, not {}", bgrx.len(), pixels * 4));
    }
    let mut rgb = Vec::with_capacity(pixels * 3);
    for p in bgrx[..pixels * 4].chunks_exact(4) {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let mut out = Cursor::new(vec![FRAME]);
    out.set_position(1);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("a frame could not be made: {e}"))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: u32, h: u32, bgrx: [u8; 4]) -> Vec<u8> {
        bgrx.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn a_screen_is_sent_once_and_again_only_when_it_changes() {
        let mut c = Changes::default();
        let a = screen(4, 4, [0, 0, 255, 0]);
        assert!(c.changed(&a), "the first screen is always sent");
        assert!(!c.changed(&a), "the same screen is not sent again");
        assert!(c.changed(&screen(4, 4, [0, 255, 0, 0])));
        assert!(c.changed(&a), "going back is a change");
    }

    #[test]
    fn a_frame_is_the_kind_byte_and_a_jpeg_of_the_right_colour_and_size() {
        // The server's order is blue, green, red: this screen is red.
        let m = frame_message(&screen(16, 8, [0, 0, 255, 0]), 16, 8).unwrap();
        assert_eq!(m[0], FRAME);
        let img = image::load_from_memory_with_format(&m[1..], image::ImageFormat::Jpeg).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (16, 8));
        let p = img.get_pixel(8, 4);
        assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "{p:?} is not red");
    }

    #[test]
    fn a_short_screen_is_refused_in_words() {
        let e = frame_message(&[0; 10], 4, 4).unwrap_err();
        assert!(e.contains("not 64"), "{e}");
    }
}
