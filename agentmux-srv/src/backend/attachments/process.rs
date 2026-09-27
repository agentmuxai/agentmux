// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sniff, decode and derive the two files an attachment needs besides its
//! original: a small thumbnail for the composer and transcript, and the
//! "send-copy" the agent actually receives.
//! SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.4.
//!
//! Everything here is synchronous and CPU-bound; callers run it on a
//! blocking thread.

use std::borrow::Cow;
use std::io::BufWriter;
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};

/// Largest side accepted before decoding (decompression-bomb guard).
pub const MAX_SIDE: u32 = 16_384;
/// Largest pixel count accepted before decoding.
pub const MAX_PIXELS: u64 = 200_000_000;
/// Long edge of the thumbnail.
pub const THUMB_EDGE: u32 = 256;
/// Upper bound on the send-copy's size. Bedrock and Vertex cap images at
/// 5 MB; the Anthropic API itself at 10 MB base64.
pub const SEND_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// What the first bytes of a file say it is. Extensions lie (a HEIC saved as
/// `.jpg` is common), so every decision below goes through this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Image(ImageFormat),
    Heic,
    Avif,
    Svg,
    NotImage,
}

pub fn sniff(head: &[u8]) -> Sniffed {
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Sniffed::Image(ImageFormat::Png);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Sniffed::Image(ImageFormat::Jpeg);
    }
    if head.starts_with(b"GIF8") {
        return Sniffed::Image(ImageFormat::Gif);
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Sniffed::Image(ImageFormat::WebP);
    }
    if head.starts_with(b"BM") && head.len() >= 14 {
        return Sniffed::Image(ImageFormat::Bmp);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return Sniffed::Image(ImageFormat::Tiff);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        let brand = &head[8..12];
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"hevm" | b"mif1" | b"msf1"
        ) {
            return Sniffed::Heic;
        }
        if matches!(brand, b"avif" | b"avis") {
            return Sniffed::Avif;
        }
    }
    let text = String::from_utf8_lossy(&head[..head.len().min(1024)]);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && trimmed.contains("<svg")) {
        return Sniffed::Svg;
    }
    Sniffed::NotImage
}

pub fn mime_of(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Bmp => "image/bmp",
        ImageFormat::Tiff => "image/tiff",
        _ => "application/octet-stream",
    }
}

pub fn ext_of(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tif",
        _ => "bin",
    }
}

/// Why an image couldn't be processed. `code` values are the
/// `AttachmentFailedEvent::code` vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError {
    pub code: &'static str,
    pub message: String,
}

impl ProcessError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One derived file, encoded in memory.
#[derive(Debug, Clone)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

/// Everything the pipeline derives from one original.
#[derive(Debug, Clone)]
pub struct Derived {
    pub width: u32,
    pub height: u32,
    pub first_frame_only: bool,
    pub thumb: Encoded,
    pub send: Encoded,
}

/// Width and height from the file header, without decoding pixels. Fails
/// with `too_large_dimensions` past the bomb guard.
pub fn header_dimensions(path: &Path) -> Result<(u32, u32), ProcessError> {
    let (w, h) = ImageReader::open(path)
        .map_err(|e| ProcessError::new("io", e.to_string()))?
        .with_guessed_format()
        .map_err(|e| ProcessError::new("io", e.to_string()))?
        .into_dimensions()
        .map_err(|e| ProcessError::new("decode", format!("Couldn't read the image: {e}")))?;
    check_dimensions(w, h)?;
    Ok((w, h))
}

/// Bytes of memory a decode of a `w`×`h` image is budgeted at: the RGBA8
/// frame plus one working copy (a resize or colour conversion).
pub fn decode_cost_bytes(w: u32, h: u32) -> u64 {
    (w as u64) * (h as u64) * 8
}

/// Decode `path` (already sniffed as `format`) and produce the thumbnail and
/// the send-copy. `send_max_edge` is the send-copy's long edge.
pub fn derive(
    path: &Path,
    format: ImageFormat,
    send_max_edge: u32,
) -> Result<Derived, ProcessError> {
    // Header-only size check before any pixel is decoded.
    header_dimensions(path)?;

    let mut reader = ImageReader::open(path).map_err(|e| ProcessError::new("io", e.to_string()))?;
    reader.set_format(format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    // 200 MP of RGBA16 is 1.6 GB; RGBA8 is 800 MB. Allow the 8-bit case.
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| ProcessError::new("decode", format!("Couldn't read the image: {e}")))?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder)
        .map_err(|e| ProcessError::new("decode", format!("Couldn't decode the image: {e}")))?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }

    let first_frame_only = format == ImageFormat::Gif && gif_is_animated(path);
    let alpha = has_real_alpha(&img);
    let lossless_source = matches!(
        format,
        ImageFormat::Png | ImageFormat::Bmp | ImageFormat::Gif | ImageFormat::Tiff
    );

    let thumb = encode_thumbnail(&img, alpha)?;
    let send = encode_send_copy(&img, alpha, lossless_source, send_max_edge.max(64))?;

    Ok(Derived {
        width: img.width(),
        height: img.height(),
        first_frame_only,
        thumb,
        send,
    })
}

pub fn check_dimensions(w: u32, h: u32) -> Result<(), ProcessError> {
    if w == 0 || h == 0 {
        return Err(ProcessError::new("decode", "The image has no pixels."));
    }
    if w > MAX_SIDE || h > MAX_SIDE || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(ProcessError::new(
            "too_large_dimensions",
            format!(
                "The image is {w}×{h}. The largest supported is {MAX_SIDE} px on a side and {} megapixels.",
                MAX_PIXELS / 1_000_000
            ),
        ));
    }
    Ok(())
}

/// True when some pixel is not fully opaque. Many screenshots carry an alpha
/// channel that is 255 everywhere; those can still become JPEG.
fn has_real_alpha(img: &DynamicImage) -> bool {
    if !img.color().has_alpha() {
        return false;
    }
    // Sample a bounded thumbnail rather than every pixel of a large image.
    let probe = if img.width().max(img.height()) > 512 {
        img.thumbnail(512, 512)
    } else {
        img.clone()
    };
    probe.to_rgba8().pixels().any(|p| p.0[3] < 255)
}

fn gif_is_animated(path: &Path) -> bool {
    use image::AnimationDecoder;
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(dec) = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(f)) else {
        return false;
    };
    dec.into_frames().take(2).count() > 1
}

fn fit(w: u32, h: u32, edge: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= edge {
        return (w, h);
    }
    let scale = edge as f64 / long as f64;
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

fn encode_thumbnail(img: &DynamicImage, alpha: bool) -> Result<Encoded, ProcessError> {
    let (tw, th) = fit(img.width(), img.height(), THUMB_EDGE);
    let small: Cow<'_, DynamicImage> = if (tw, th) == (img.width(), img.height()) {
        Cow::Borrowed(img)
    } else {
        Cow::Owned(img.thumbnail(tw, th))
    };
    if alpha {
        encode_png(&small)
    } else {
        encode_jpeg(&small, 80)
    }
}

fn encode_send_copy(
    img: &DynamicImage,
    alpha: bool,
    lossless_source: bool,
    max_edge: u32,
) -> Result<Encoded, ProcessError> {
    let mut edge = max_edge;
    // Shrink by 20% per attempt until the file fits. Four attempts take a
    // 2000 px image down to ~820 px, which fits any realistic content.
    for _ in 0..5 {
        let (sw, sh) = fit(img.width(), img.height(), edge);
        // Borrow at full size: cloning a large decoded frame would double
        // the memory this job was budgeted for.
        let scaled: Cow<'_, DynamicImage> = if (sw, sh) == (img.width(), img.height()) {
            Cow::Borrowed(img)
        } else {
            Cow::Owned(img.resize_exact(sw, sh, FilterType::CatmullRom))
        };
        let out = if alpha {
            encode_png(&scaled)?
        } else if lossless_source {
            // Screenshots and UI captures stay sharp as PNG; fall back to
            // JPEG only when PNG is too big (photos saved as PNG).
            let png = encode_png(&scaled)?;
            if png.bytes.len() as u64 <= SEND_MAX_BYTES {
                png
            } else {
                encode_jpeg(&scaled, 85)?
            }
        } else {
            encode_jpeg(&scaled, 85)?
        };
        if out.bytes.len() as u64 <= SEND_MAX_BYTES {
            return Ok(out);
        }
        edge = ((sw.max(sh) as f64) * 0.8) as u32;
    }
    Err(ProcessError::new(
        "decode",
        "Couldn't make a small enough copy of this image to send.",
    ))
}

fn encode_png(img: &DynamicImage) -> Result<Encoded, ProcessError> {
    // 16-bit and float images become 8-bit: no model uses the extra depth.
    let rgba;
    let rgb;
    let (buf, color): (&[u8], image::ExtendedColorType) = if img.color().has_alpha() {
        rgba = img.to_rgba8();
        (rgba.as_raw(), image::ExtendedColorType::Rgba8)
    } else {
        rgb = img.to_rgb8();
        (rgb.as_raw(), image::ExtendedColorType::Rgb8)
    };
    let mut out = Vec::new();
    PngEncoder::new(BufWriter::new(&mut out))
        .write_image(buf, img.width(), img.height(), color)
        .map_err(|e| ProcessError::new("decode", format!("Couldn't encode PNG: {e}")))?;
    Ok(Encoded {
        bytes: out,
        format: ImageFormat::Png,
        width: img.width(),
        height: img.height(),
    })
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Encoded, ProcessError> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(
            rgb.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| ProcessError::new("decode", format!("Couldn't encode JPEG: {e}")))?;
    Ok(Encoded {
        bytes: out,
        format: ImageFormat::Jpeg,
        width: img.width(),
        height: img.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn write(dir: &Path, name: &str, img: &DynamicImage, fmt: ImageFormat) -> std::path::PathBuf {
        let p = dir.join(name);
        img.save_with_format(&p, fmt).unwrap();
        p
    }

    #[test]
    fn sniff_recognises_formats_by_content_not_name() {
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 0, 0]),
            Sniffed::Image(ImageFormat::Png)
        );
        assert_eq!(
            sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Sniffed::Image(ImageFormat::Jpeg)
        );
        assert_eq!(sniff(b"GIF89a......"), Sniffed::Image(ImageFormat::Gif));
        assert_eq!(
            sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Sniffed::Image(ImageFormat::WebP)
        );
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0"), Sniffed::Heic);
        assert_eq!(sniff(b"\0\0\0\x18ftypmif1\0\0\0\0"), Sniffed::Heic);
        assert_eq!(sniff(b"\0\0\0\x1cftypavif\0\0\0\0"), Sniffed::Avif);
        assert_eq!(sniff(b"\xef\xbb\xbf  <svg xmlns=..."), Sniffed::Svg);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><svg>"), Sniffed::Svg);
        assert_eq!(sniff(b"fn main() {}"), Sniffed::NotImage);
        assert_eq!(sniff(b""), Sniffed::NotImage);
    }

    #[test]
    fn dimension_guard_rejects_bombs_before_decode() {
        assert!(check_dimensions(4000, 3000).is_ok());
        assert_eq!(
            check_dimensions(20_000, 10).unwrap_err().code,
            "too_large_dimensions"
        );
        assert_eq!(
            check_dimensions(16_000, 16_000).unwrap_err().code,
            "too_large_dimensions"
        );
        assert_eq!(check_dimensions(0, 10).unwrap_err().code, "decode");
    }

    #[test]
    fn large_photo_is_downscaled_to_the_send_edge_as_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(3000, 1500, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        }));
        let p = write(dir.path(), "photo.jpg", &img, ImageFormat::Jpeg);
        let d = derive(&p, ImageFormat::Jpeg, 2000).unwrap();
        assert_eq!((d.width, d.height), (3000, 1500));
        assert_eq!(d.send.format, ImageFormat::Jpeg);
        assert_eq!((d.send.width, d.send.height), (2000, 1000));
        assert!(d.send.bytes.len() as u64 <= SEND_MAX_BYTES);
        assert_eq!(d.thumb.width.max(d.thumb.height), THUMB_EDGE);
    }

    #[test]
    fn small_screenshot_stays_png_at_full_size() {
        let dir = tempfile::tempdir().unwrap();
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(800, 600, Rgba([30, 30, 30, 255])));
        let p = write(dir.path(), "shot.png", &img, ImageFormat::Png);
        let d = derive(&p, ImageFormat::Png, 2000).unwrap();
        assert_eq!(d.send.format, ImageFormat::Png);
        assert_eq!((d.send.width, d.send.height), (800, 600));
        // Fully opaque alpha doesn't force a PNG thumbnail.
        assert_eq!(d.thumb.format, ImageFormat::Jpeg);
    }

    #[test]
    fn transparency_is_kept_as_png() {
        let dir = tempfile::tempdir().unwrap();
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(300, 300, |x, _| {
            Rgba([255, 0, 0, if x < 150 { 0 } else { 255 }])
        }));
        let p = write(dir.path(), "logo.png", &img, ImageFormat::Png);
        let d = derive(&p, ImageFormat::Png, 2000).unwrap();
        assert_eq!(d.send.format, ImageFormat::Png);
        assert_eq!(d.thumb.format, ImageFormat::Png);
    }

    #[test]
    fn send_copy_carries_no_exif() {
        let dir = tempfile::tempdir().unwrap();
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, Rgb([1, 2, 3])));
        let p = write(dir.path(), "a.jpg", &img, ImageFormat::Jpeg);
        let d = derive(&p, ImageFormat::Jpeg, 2000).unwrap();
        // An APP1 "Exif" segment would appear right after SOI.
        assert!(!d.send.bytes.windows(4).any(|w| w == b"Exif"));
    }

    #[test]
    fn truncated_file_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 200, Rgb([9, 9, 9])));
        let p = write(dir.path(), "t.png", &img, ImageFormat::Png);
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() / 3]).unwrap();
        let e = derive(&p, ImageFormat::Png, 2000).unwrap_err();
        assert_eq!(e.code, "decode");
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit(4000, 2000, 2000), (2000, 1000));
        assert_eq!(fit(100, 50, 2000), (100, 50));
        assert_eq!(fit(1, 5000, 256), (1, 256));
    }
}
