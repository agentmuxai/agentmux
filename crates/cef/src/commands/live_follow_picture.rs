// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pixels of the picture that follows the cursor during a drag (live_follow.rs): resized to the size it
// is shown at, and, for a pane, cut out of a picture of its window once the pane stands in its landing rect.
// Nothing here knows a window or a lock: slices in, pixels out.

/// The picture at `size`, from `source` (BGRA at `source_size`): the whole picture, resized. Each pixel is the
/// mean of the source pixels it covers, so a pane made smaller stays readable. A source of other proportions
/// than `size` is stretched to it; a source shorter than its size paints black.
pub(crate) fn compose(source: &[u8], source_size: (i32, i32), size: (i32, i32)) -> Vec<u8> {
    let (sw, sh) = (source_size.0.max(0) as usize, source_size.1.max(0) as usize);
    let (w, h) = (size.0.max(0) as usize, size.1.max(0) as usize);
    if sw == 0 || sh == 0 || source.len() < sw * sh * 4 {
        return [0, 0, 0, 255].repeat(w * h);
    }
    if (sw, sh) == (w, h) {
        return source[..w * h * 4].to_vec();
    }
    // The source columns and rows each output pixel covers: at least one, so a larger size repeats pixels.
    let span = |i: usize, from: usize, to: usize| {
        let start = i * from / to;
        let end = ((i + 1) * from).div_ceil(to).clamp(start + 1, from);
        (start.min(from - 1), end)
    };
    let cols: Vec<(usize, usize)> = (0..w).map(|x| span(x, sw, w)).collect();
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let (y0, y1) = span(y, sh, h);
        for &(x0, x1) in &cols {
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                let row = &source[(sy * sw + x0) * 4..(sy * sw + x1) * 4];
                for px in row.chunks_exact(4) {
                    sum[0] += px[0] as u32;
                    sum[1] += px[1] as u32;
                    sum[2] += px[2] as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    out
}

/// A rect of the page (left, top, width, height, in its CSS px) in the pixels of a picture of the page's whole
/// viewport, which is `viewport_width` CSS px wide: scaled by the picture's width over the viewport's (the
/// device pixel ratio and the zoom in one), and kept inside the picture. None when nothing of it is left.
pub(crate) fn crop_rect(rect: (f64, f64, f64, f64), viewport_width: f64, image: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    if !(viewport_width > 0.0) || image.0 == 0 || image.1 == 0 {
        return None;
    }
    let scale = image.0 as f64 / viewport_width;
    let edge = |css: f64, max: u32| ((css * scale).round().max(0.0) as u32).min(max);
    let (x0, y0) = (edge(rect.0, image.0), edge(rect.1, image.1));
    let (x1, y1) = (edge(rect.0 + rect.2, image.0), edge(rect.1 + rect.3, image.1));
    (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

/// The landing zone's fill over a pane: a flat colour at `alpha`, drawn over every pixel of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wash {
    pub rgb: (u8, u8, u8),
    pub alpha: f64,
}

/// The pixels of `rect` (x, y, width, height) of an RGB picture `image_width` wide, as opaque BGRA. With a
/// `wash`, each pixel is what the wash was drawn over: the pane in its landing rect is on screen under the
/// landing zone's fill, the picture is taken through it, and the fill is a known flat colour at a known
/// strength, so `shown = pane * (1 - alpha) + wash * alpha` is solved for the pane. A rect that reaches past
/// the picture gives None.
pub(crate) fn crop_bgra(rgb: &[u8], image_width: u32, rect: (u32, u32, u32, u32), wash: Option<Wash>) -> Option<Vec<u8>> {
    let (iw, (x, y, w, h)) = (image_width as usize, (rect.0 as usize, rect.1 as usize, rect.2 as usize, rect.3 as usize));
    if w == 0 || h == 0 || x + w > iw || rgb.len() < (y + h) * iw * 3 {
        return None;
    }
    // One table per channel: what a shown value was before the wash.
    let table = |tint: u8| -> [u8; 256] {
        let mut t = [0u8; 256];
        for (shown, slot) in t.iter_mut().enumerate() {
            *slot = match wash {
                Some(Wash { alpha, .. }) if alpha > 0.0 && alpha < 1.0 => {
                    ((shown as f64 - tint as f64 * alpha) / (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
                }
                _ => shown as u8,
            };
        }
        t
    };
    let tint = wash.map_or((0, 0, 0), |w| w.rgb);
    let (tr, tg, tb) = (table(tint.0), table(tint.1), table(tint.2));
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let line = &rgb[(row * iw + x) * 3..(row * iw + x + w) * 3];
        for px in line.chunks_exact(3) {
            out.extend_from_slice(&[tb[px[2] as usize], tg[px[1] as usize], tr[px[0] as usize], 255]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{compose, crop_bgra, crop_rect, Wash};

    /// A 4 by 2 BGRA picture whose pixel (x, y) is (10x, 100y, 7, 255), so a resize is read back by value.
    fn picture() -> Vec<u8> {
        (0..2).flat_map(|y| (0..4).map(move |x| [10 * x as u8, 100 * y as u8, 7, 255])).flatten().collect()
    }

    #[test]
    fn a_smaller_size_is_the_whole_picture_resized_not_its_top_left_corner() {
        // Half the width, half the height: each pixel is the mean of the 2 by 2 it covers.
        let out = compose(&picture(), (4, 2), (2, 1));
        assert_eq!(out, vec![5, 50, 7, 255, 25, 50, 7, 255]);
    }

    #[test]
    fn the_panes_own_size_is_the_picture_as_it_is() {
        assert_eq!(compose(&picture(), (4, 2), (4, 2)), picture());
    }

    #[test]
    fn a_larger_size_repeats_pixels_and_fills_every_one() {
        let out = compose(&picture(), (4, 2), (8, 4));
        assert_eq!(out.len(), 8 * 4 * 4);
        // Row 0: each source pixel of the first row, twice.
        assert_eq!(&out[0..16], &[0, 0, 7, 255, 0, 0, 7, 255, 10, 0, 7, 255, 10, 0, 7, 255]);
        // The last row is the source's last row.
        assert_eq!(&out[3 * 8 * 4..3 * 8 * 4 + 4], &[0, 100, 7, 255]);
    }

    #[test]
    fn a_short_source_paints_black() {
        let out = compose(&[1, 2, 3], (3, 2), (1, 1));
        assert_eq!(out, vec![0, 0, 0, 255]);
    }

    #[test]
    fn a_rect_of_the_page_is_found_in_a_picture_at_twice_its_scale_and_kept_inside_it() {
        // A 1000 px wide viewport pictured 2000 px wide (a 200% display).
        assert_eq!(crop_rect((100.0, 50.0, 300.0, 200.0), 1000.0, (2000, 1200)), Some((200, 100, 600, 400)));
        // Reaching past the right and bottom edges: cut at them.
        assert_eq!(crop_rect((900.0, 500.0, 300.0, 200.0), 1000.0, (2000, 1200)), Some((1800, 1000, 200, 200)));
        // Starting before the left edge: from 0.
        assert_eq!(crop_rect((-50.0, 0.0, 100.0, 10.0), 1000.0, (2000, 1200)), Some((0, 0, 100, 20)));
        // Outside, or with no viewport to scale by: nothing.
        assert_eq!(crop_rect((1200.0, 0.0, 100.0, 100.0), 1000.0, (2000, 1200)), None);
        assert_eq!(crop_rect((0.0, 0.0, 100.0, 100.0), 0.0, (2000, 1200)), None);
    }

    /// A 3 by 2 RGB picture whose pixel (x, y) is (x, 10y, 200).
    fn rgb() -> Vec<u8> {
        (0..2).flat_map(|y| (0..3).map(move |x| [x as u8, 10 * y as u8, 200])).flatten().collect()
    }

    #[test]
    fn a_crop_is_the_rects_own_pixels_as_opaque_bgra() {
        // The right two columns of the second row.
        assert_eq!(crop_bgra(&rgb(), 3, (1, 1, 2, 1), None), Some(vec![200, 10, 1, 255, 200, 10, 2, 255]));
        // A rect past the picture is refused, not read out of bounds.
        assert_eq!(crop_bgra(&rgb(), 3, (2, 0, 2, 1), None), None);
        assert_eq!(crop_bgra(&rgb(), 3, (0, 1, 1, 2), None), None);
    }

    #[test]
    fn a_wash_is_taken_back_out_of_the_pixels_it_was_drawn_over() {
        let wash = Wash { rgb: (65, 159, 224), alpha: 0.26 };
        let over = |pane: u8, tint: u8| (pane as f64 * 0.74 + tint as f64 * 0.26).round() as u8;
        // Panes of a few colours, each as the screen shows it under the wash.
        for pane in [(0u8, 0u8, 0u8), (255, 255, 255), (30, 30, 46), (200, 120, 10)] {
            let shown = [over(pane.0, 65), over(pane.1, 159), over(pane.2, 224)];
            let out = crop_bgra(&shown, 1, (0, 0, 1, 1), Some(wash)).unwrap();
            // Within a level of the pane's own colour (the screen's value was rounded once).
            for (got, want) in [(out[2], pane.0), (out[1], pane.1), (out[0], pane.2)] {
                assert!((got as i32 - want as i32).abs() <= 1, "{:?} came back as {:?}", pane, out);
            }
            assert_eq!(out[3], 255);
        }
        // A wash of no strength, or a full one (nothing of the pane is left to solve for), changes nothing.
        for alpha in [0.0, 1.0] {
            let same = crop_bgra(&[9, 8, 7], 1, (0, 0, 1, 1), Some(Wash { rgb: (65, 159, 224), alpha }));
            assert_eq!(same, Some(vec![7, 8, 9, 255]));
        }
    }
}
