// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A frame of the virtual screen as the pane wants it: the same kind byte and JPEG the streamed browser sends
//! (`pane.rs`), made only when the screen changed since the last one that was sent. The screen says which part of it changed
//! (the damage extension), so only that part is read; the picture of the whole screen is kept here and each change is written into it.

use std::io::Cursor;

/// The first byte of a frame message (the pane's `FRAME`).
pub const FRAME: u8 = 1;
/// The same quality as the streamed browser's screencast.
const JPEG_QUALITY: u8 = 70;

/// A part of the screen, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    /// The whole screen.
    pub fn whole(w: u16, h: u16) -> Self {
        Self { x: 0, y: 0, w, h }
    }

    /// A rectangle as the damage extension reports it, cut to a screen of `screen_w` by `screen_h`; none when nothing of it is on the screen.
    pub fn clipped(x: i16, y: i16, w: u16, h: u16, screen_w: u16, screen_h: u16) -> Option<Self> {
        let (x0, y0) = (i32::from(x).max(0), i32::from(y).max(0));
        let (x1, y1) = ((i32::from(x) + i32::from(w)).min(i32::from(screen_w)), (i32::from(y) + i32::from(h)).min(i32::from(screen_h)));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Self { x: x0 as u16, y: y0 as u16, w: (x1 - x0) as u16, h: (y1 - y0) as u16 })
    }

    /// The smallest rectangle that holds both.
    pub fn union(self, other: Self) -> Self {
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let (x1, y1) = ((self.x + self.w).max(other.x + other.w), (self.y + self.h).max(other.y + other.h));
        Self { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// The picture of the whole screen as last read: four bytes a pixel (blue, green, red, one unused), row after row. A part that was read
/// again is written in, and says whether it was different.
pub struct Picture {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Picture {
    pub fn new(w: u16, h: u16) -> Self {
        Self { w: usize::from(w), h: usize::from(h), px: vec![0; usize::from(w) * usize::from(h) * 4] }
    }

    /// The whole picture.
    pub fn pixels(&self) -> &[u8] {
        &self.px
    }

    /// Write `part` (the pixels of `at`, row after row) into the picture. True when any pixel of that part was different; a part that does not
    /// fit the picture, or came back short, is not written and is not a change.
    pub fn update(&mut self, at: Rect, part: &[u8]) -> bool {
        let (x, y, w, h) = (usize::from(at.x), usize::from(at.y), usize::from(at.w), usize::from(at.h));
        if x + w > self.w || y + h > self.h || part.len() < w * h * 4 {
            return false;
        }
        let mut changed = false;
        for row in 0..h {
            let (from, to) = (row * w * 4, ((y + row) * self.w + x) * 4);
            let (new, old) = (&part[from..from + w * 4], &mut self.px[to..to + w * 4]);
            if new != old {
                old.copy_from_slice(new);
                changed = true;
            }
        }
        changed
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
    fn a_part_that_is_the_same_is_no_change_and_one_that_differs_is_written_in() {
        let mut p = Picture::new(4, 4);
        let all = Rect::whole(4, 4);
        assert!(p.update(all, &screen(4, 4, [0, 0, 255, 0])), "the first read of a coloured screen is a change from the blank picture");
        assert!(!p.update(all, &screen(4, 4, [0, 0, 255, 0])), "the same screen is not a change");
        let part = Rect { x: 1, y: 2, w: 2, h: 1 };
        assert!(p.update(part, &screen(2, 1, [0, 255, 0, 0])));
        assert!(!p.update(part, &screen(2, 1, [0, 255, 0, 0])));
        let px = p.pixels();
        let at = |x: usize, y: usize| &px[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3];
        assert_eq!(at(1, 2), [0, 255, 0], "the part is written where it was read");
        assert_eq!(at(2, 2), [0, 255, 0]);
        assert_eq!(at(0, 2), [0, 0, 255], "what is around it is untouched");
        assert_eq!(at(3, 2), [0, 0, 255]);
        assert_eq!(at(1, 1), [0, 0, 255]);
    }

    #[test]
    fn a_part_that_does_not_fit_or_came_back_short_is_ignored() {
        let mut p = Picture::new(4, 4);
        assert!(!p.update(Rect { x: 3, y: 3, w: 2, h: 2 }, &screen(2, 2, [1, 2, 3, 0])), "off the screen");
        assert!(!p.update(Rect::whole(4, 4), &[1, 2, 3]), "short");
        assert!(p.pixels().iter().all(|b| *b == 0));
    }

    #[test]
    fn damage_is_cut_to_the_screen_and_joined_into_one_rectangle() {
        assert_eq!(Rect::clipped(-5, -5, 10, 10, 100, 80), Some(Rect { x: 0, y: 0, w: 5, h: 5 }));
        assert_eq!(Rect::clipped(90, 70, 50, 50, 100, 80), Some(Rect { x: 90, y: 70, w: 10, h: 10 }));
        assert_eq!(Rect::clipped(100, 0, 10, 10, 100, 80), None, "right of the screen");
        assert_eq!(Rect::clipped(0, 0, 0, 10, 100, 80), None, "empty");
        let a = Rect { x: 10, y: 10, w: 5, h: 5 };
        let b = Rect { x: 30, y: 2, w: 4, h: 4 };
        assert_eq!(a.union(b), Rect { x: 10, y: 2, w: 24, h: 13 });
        assert_eq!(a.union(a), a);
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
