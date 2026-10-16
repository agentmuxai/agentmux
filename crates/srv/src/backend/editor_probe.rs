// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a file is, for the Editor to pick a viewer before it reads anything:
//! one stat and the first 8 KB. Content decides first (magic bytes, then
//! text vs binary), the extension only settles what content leaves open.
//!
//! The attachment classifier (`attachments::kind`) supplies the magic-byte
//! table. Its text test ("valid UTF-8, no NUL") is too strict for an editor
//! — a CP1252 `.ini` file is text — so the text/binary rule here is its own:
//! a BOM, then the UTF-16 NUL-parity test, then any NUL, then the share of
//! WHATWG "binary data bytes".
//!
//! docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2, §5.3.

use std::io::Read;
use std::path::Path;

use image::ImageFormat;

use crate::backend::attachments::kind::{self as attach_kind, FileKind};
use crate::backend::attachments::process::{self, Sniffed};
use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{FileProbeReq, FileProbeResult, FileProbeText};

/// How many bytes the probe reads from the start of the file.
pub const PROBE_BYTES: usize = 8192;
/// The UTF-16 parity test looks at this many bytes (VS Code's window).
const UTF16_WINDOW: usize = 512;

/// The text side of a probe: how the file is encoded, as far as the first
/// bytes tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedText {
    /// "utf-8", "utf-16le", "utf-16be", or "legacy" (not UTF-8; the read
    /// path's detector decides which single-byte or CJK encoding).
    pub encoding: &'static str,
    /// The BOM present: "none" | "utf-8" | "utf-16le" | "utf-16be".
    pub bom: &'static str,
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// One of: image, image_file, svg, pdf, audio, video, archive, word,
    /// excel, powerpoint, sqlite, font, text, other.
    pub kind: &'static str,
    pub mime: &'static str,
    /// Some for text, None for binary.
    pub text: Option<ProbedText>,
    /// Why binary: "nul" or "control_bytes"; None for text, and for a
    /// binary format recognised by its magic bytes.
    pub binary_reason: Option<&'static str>,
}

/// `fileprobe`: what a file is (kind, MIME, text or binary), from one stat
/// and its first 8 KB, so the Editor picks a viewer before reading it. This
/// computer's files only: an editor on an SSH host reads as text.
pub fn register(engine: &std::sync::Arc<WshRpcEngine>) {
    engine.register_typed("fileprobe", move |cmd: FileProbeReq, _ctx| async move {
        let path = expand_home_dir_safe(&cmd.path);
        tokio::task::spawn_blocking(move || {
            let (meta, probe) = probe_path(&path).map_err(|e| format!("fileprobe: {e}"))?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Ok(FileProbeResult {
                size: meta.len(),
                mtime,
                kind: probe.kind.to_string(),
                mime: probe.mime.to_string(),
                text: probe.text.map(|t| FileProbeText { encoding: t.encoding.to_string(), bom: t.bom.to_string() }),
                binary_reason: probe.binary_reason.map(str::to_string),
                read_only: meta.permissions().readonly(),
            })
        })
        .await
        .map_err(|e| format!("fileprobe: {e}"))?
    });
}

/// Probe the file at `path`.
pub fn probe_path(path: &Path) -> std::io::Result<(std::fs::Metadata, Probe)> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(std::io::Error::other("is a directory"));
    }
    let mut head = Vec::with_capacity(PROBE_BYTES);
    std::fs::File::open(path)?.take(PROBE_BYTES as u64).read_to_end(&mut head)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = attach_kind::extension_of(name);
    let probe = probe_bytes(&head, ext.as_deref(), || {
        // The OOXML/ODF inspection opens the ZIP: only for ZIP magic.
        let k = attach_kind::classify(path, name).ok()?;
        matches!(k, FileKind::Word | FileKind::Excel | FileKind::PowerPoint).then_some(k)
    });
    Ok((meta, probe))
}

/// [`probe_path`] on the first bytes, with the ZIP inspection injected (tests).
pub fn probe_bytes(head: &[u8], ext: Option<&str>, zip_kind: impl FnOnce() -> Option<FileKind>) -> Probe {
    let binary = |kind: &'static str, mime: &'static str| Probe { kind, mime, text: None, binary_reason: None };

    // 1. Images by magic. Chromium decodes these itself; TIFF and HEIC it
    //    doesn't (they get the info card until a converter exists). BMP's
    //    "BM" and GIF's "GIF8" are too short to trust alone: a text file can
    //    begin with them, so those need their full header.
    match process::sniff(head) {
        Sniffed::Image(ImageFormat::Tiff) => return binary("image_file", "image/tiff"),
        Sniffed::Image(ImageFormat::Bmp) if !is_bmp(head) => {}
        Sniffed::Image(ImageFormat::Gif) if !(head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a")) => {}
        Sniffed::Image(f) => return binary("image", image_mime(f, head)),
        Sniffed::Heic => return binary("image_file", "image/heic"),
        Sniffed::Avif => return binary("image", "image/avif"),
        Sniffed::Svg => {
            return Probe { kind: "svg", mime: "image/svg+xml", text: text_info(head).ok(), binary_reason: None }
        }
        Sniffed::NotImage => {}
    }
    // ICO starts with NULs, so it has to be checked before the text test.
    if head.len() >= 6 && head[..4] == [0, 0, 1, 0] && head[4..6] != [0, 0] {
        return binary("image", "image/x-icon");
    }
    // JPEG XL: a bare codestream, or the ISO-BMFF container.
    if head.starts_with(&[0xFF, 0x0A]) || head.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n") {
        return binary("image_file", "image/jxl");
    }
    // A PDF starts with its header. Readers also accept up to 1 KB of junk
    // before it, but a text file mentioning "%PDF-" near its top is text:
    // the late header counts only when the bytes aren't text.
    if head.starts_with(b"%PDF-")
        || (text_info(head).is_err() && head[..head.len().min(1024)].windows(5).any(|w| w == b"%PDF-"))
    {
        return binary("pdf", "application/pdf");
    }
    if head.starts_with(b"SQLite format 3\0") {
        return binary("sqlite", "application/vnd.sqlite3");
    }
    if let Some(mime) = font_mime(head, ext) {
        return binary("font", mime);
    }

    // 2. Text or binary.
    let reason = match text_info(head) {
        // Text bytes are text, whatever the name says (`.ts` is also MPEG
        // transport stream; a `.mp3` that holds words is words).
        Ok(t) => {
            let mime = if ext.is_some_and(|e| e == "csv") { "text/csv" } else { "text/plain" };
            return Probe { kind: "text", mime, text: Some(t), binary_reason: None };
        }
        Err(reason) => Some(reason),
    };

    // 3. Binary: the attachment classifier's magic table, then the extension.
    let kind = match attach_kind::classify_bytes(head, ext, zip_kind) {
        FileKind::Word => "word",
        FileKind::Excel => "excel",
        FileKind::PowerPoint => "powerpoint",
        FileKind::Archive => "archive",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        FileKind::Pdf => "pdf",
        FileKind::Image(_) | FileKind::ImageFile => "image_file",
        FileKind::Svg | FileKind::Text | FileKind::Other => "other",
    };
    Probe { kind, mime: binary_mime(kind, head, ext), text: None, binary_reason: reason }
}

/// Text, with its encoding, or the reason it is binary.
fn text_info(head: &[u8]) -> Result<ProbedText, &'static str> {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok(ProbedText { encoding: "utf-8", bom: "utf-8" });
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return Ok(ProbedText { encoding: "utf-16le", bom: "utf-16le" });
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return Ok(ProbedText { encoding: "utf-16be", bom: "utf-16be" });
    }
    if let Some(enc) = utf16_by_parity(&head[..head.len().min(UTF16_WINDOW)]) {
        return Ok(ProbedText { encoding: enc, bom: "none" });
    }
    if head.contains(&0) {
        return Err("nul");
    }
    let control = head.iter().filter(|&&b| is_binary_data_byte(b)).count();
    // More than 1 in 100 bytes (and at least 2) is not a text file with a
    // stray bell or form feed in it.
    if control >= 2 && control * 100 > head.len() {
        return Err("control_bytes");
    }
    let encoding = match std::str::from_utf8(head) {
        Ok(_) => "utf-8",
        // A code point cut by the 8 KB window is still UTF-8.
        Err(e) if e.error_len().is_none() => "utf-8",
        Err(_) => "legacy",
    };
    Ok(ProbedText { encoding, bom: "none" })
}

/// UTF-16 without a BOM: ASCII-range text puts a NUL in every other byte,
/// on odd offsets for little-endian and even ones for big-endian.
fn utf16_by_parity(window: &[u8]) -> Option<&'static str> {
    let pairs = window.len() / 2;
    if pairs < 2 {
        return None;
    }
    let (mut even, mut odd) = (0usize, 0usize);
    for (i, &b) in window[..pairs * 2].iter().enumerate() {
        if b == 0 {
            if i % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    // Most pairs carry the NUL, and never on the other parity.
    let most = |n: usize| n * 10 >= pairs * 7;
    if even == 0 && most(odd) {
        Some("utf-16le")
    } else if odd == 0 && most(even) {
        Some("utf-16be")
    } else {
        None
    }
}

/// WHATWG MIME Sniffing §7.1: a byte that never appears in text.
fn is_binary_data_byte(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F)
}

fn image_mime(f: ImageFormat, head: &[u8]) -> &'static str {
    match f {
        ImageFormat::Png if is_apng(head) => "image/apng",
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Bmp => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// A BMP file header: "BM", then a DIB header whose size (at offset 14) is
/// one of the known versions (OS/2 1.x, Windows 3.x, the V2–V5 extensions).
fn is_bmp(head: &[u8]) -> bool {
    head.len() >= 18
        && head.starts_with(b"BM")
        && matches!(u32::from_le_bytes([head[14], head[15], head[16], head[17]]), 12 | 16 | 40 | 52 | 56 | 64 | 108 | 124)
}

/// An animated PNG has an `acTL` chunk before its first `IDAT`.
fn is_apng(head: &[u8]) -> bool {
    let actl = head.windows(4).position(|w| w == b"acTL");
    let idat = head.windows(4).position(|w| w == b"IDAT");
    matches!((actl, idat), (Some(a), Some(i)) if a < i) || matches!((actl, idat), (Some(_), None))
}

fn font_mime(head: &[u8], ext: Option<&str>) -> Option<&'static str> {
    if head.starts_with(b"wOFF") {
        return Some("font/woff");
    }
    if head.starts_with(b"wOF2") {
        return Some("font/woff2");
    }
    if head.starts_with(b"OTTO") {
        return Some("font/otf");
    }
    if head.starts_with(b"ttcf") {
        return Some("font/collection");
    }
    // `00 01 00 00` opens TrueType, but other formats too: trust it with a
    // font extension only.
    if head.starts_with(&[0, 1, 0, 0]) && matches!(ext, Some("ttf") | Some("otf") | Some("ttc")) {
        return Some("font/ttf");
    }
    None
}

fn binary_mime(kind: &str, head: &[u8], ext: Option<&str>) -> &'static str {
    match (kind, ext) {
        ("audio", Some("mp3")) => "audio/mpeg",
        ("audio", Some("wav")) => "audio/wav",
        ("audio", Some("flac")) => "audio/flac",
        ("audio", Some("ogg") | Some("oga") | Some("opus")) => "audio/ogg",
        ("audio", Some("m4a") | Some("aac")) => "audio/mp4",
        ("audio", _) if head.starts_with(b"ID3") => "audio/mpeg",
        ("audio", _) if head.starts_with(b"fLaC") => "audio/flac",
        ("audio", _) if head.starts_with(b"OggS") => "audio/ogg",
        ("audio", _) => "audio/wav",
        ("video", Some("webm") | Some("mkv")) => "video/webm",
        ("video", Some("mov")) => "video/quicktime",
        ("video", _) if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) => "video/webm",
        ("video", _) => "video/mp4",
        ("archive", Some("zip")) => "application/zip",
        ("archive", _) if head.starts_with(b"PK") => "application/zip",
        ("archive", Some("gz") | Some("tgz")) => "application/gzip",
        ("word", _) => "application/msword",
        ("excel", _) => "application/vnd.ms-excel",
        ("powerpoint", _) => "application/vnd.ms-powerpoint",
        ("pdf", _) => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(head: &[u8], ext: Option<&str>) -> Probe {
        probe_bytes(head, ext, || None)
    }

    #[tokio::test]
    async fn fileprobe_records_its_types() {
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine);
        let schema = engine.schema_json();
        let row = schema
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["command"] == "fileprobe")
            .expect("fileprobe missing from the schema");
        assert_eq!(row["requestName"], "FileProbeReq");
        assert_eq!(row["responseName"], "FileProbeResult");
    }

    #[test]
    fn probes_a_real_file_and_refuses_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.png");
        std::fs::write(&file, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]).unwrap();
        let (meta, probe) = probe_path(&file).unwrap();
        assert_eq!(meta.len(), 10);
        assert_eq!(probe.kind, "image");
        assert!(probe_path(dir.path()).is_err());
    }

    #[test]
    fn images_by_magic() {
        assert_eq!(p(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0], Some("txt")).kind, "image");
        assert_eq!(p(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10], Some("png")).mime, "image/jpeg");
        assert_eq!(p(b"GIF89a\x01\0\x01\0", None).mime, "image/gif");
        assert_eq!(p(b"RIFF\0\0\0\0WEBPVP8 ", None).mime, "image/webp");
        assert_eq!(p(b"BM\x36\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0\x01\0", None).mime, "image/bmp");
        assert_eq!(p(&[0, 0, 1, 0, 1, 0, 16, 16], Some("ico")).mime, "image/x-icon");
        assert_eq!(p(b"\0\0\0\x1cftypavif\0\0\0\0", None).kind, "image");
        assert_eq!(p(b"\0\0\0\x18ftypheic\0\0\0\0", None).kind, "image_file");
        assert_eq!(p(b"II*\0\x08\0\0\0", None).kind, "image_file");
        assert_eq!(p(&[0xFF, 0x0A, 0xFA], None).mime, "image/jxl");
    }

    #[test]
    fn text_starting_like_a_weak_magic_stays_text() {
        // "BM" and "GIF8" alone don't make an image (muxreview P1 on #5016).
        assert_eq!(p(b"BMW owners club minutes, 2026\n", Some("txt")).kind, "text");
        assert_eq!(p(b"GIF8 notes: the format\n", Some("md")).kind, "text");
        // "%PDF-" in text is text; at the start, or after binary junk, it's a PDF.
        assert_eq!(p(b"// parses files that start with %PDF-1.7\n", Some("rs")).kind, "text");
        assert_eq!(p(b"\0\x01junk\0%PDF-1.4\n", None).kind, "pdf");
    }

    #[test]
    fn apng_is_told_from_png() {
        let mut head = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        head.extend_from_slice(b"\0\0\0\x0dIHDR.............\0\0\0\x08acTL........\0\0\0\0IDAT");
        assert_eq!(p(&head, None).mime, "image/apng");
    }

    #[test]
    fn svg_is_svg_and_text() {
        let r = p(b"<?xml version=\"1.0\"?><svg xmlns='http://www.w3.org/2000/svg'/>", Some("svg"));
        assert_eq!(r.kind, "svg");
        assert!(r.text.is_some());
    }

    #[test]
    fn documents_and_containers() {
        assert_eq!(p(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n", None).kind, "pdf");
        assert_eq!(p(b"SQLite format 3\0\x10\0", Some("db")).kind, "sqlite");
        assert_eq!(p(b"wOF2\0\x01\0\0", None).kind, "font");
        assert_eq!(p(&[0, 1, 0, 0, 0, 0x10], Some("ttf")).kind, "font");
        assert_eq!(p(b"PK\x03\x04\x14\0\0\0", Some("zip")).kind, "archive");
        assert_eq!(probe_bytes(b"PK\x03\x04\x14\0\0\0", Some("docx"), || Some(FileKind::Word)).kind, "word");
        assert_eq!(p(b"ID3\x04\0\0\0\0\0\0", Some("mp3")).mime, "audio/mpeg");
        assert_eq!(p(b"RIFF\x24\0\0\0WAVEfmt ", None).kind, "audio");
        assert_eq!(p(b"\0\0\0\x20ftypisom\0\0\x02\0", Some("mp4")).kind, "video");
        assert_eq!(p(b"\0\0\0\x20ftypM4A \0\0\x02\0", Some("m4a")).kind, "audio");
    }

    #[test]
    fn executables_and_unknown_binary_are_other() {
        let r = p(b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xff\xff", Some("exe"));
        assert_eq!(r.kind, "other");
        assert_eq!(r.binary_reason, Some("nul"));
        let r = p(b"\x7fELF\x02\x01\x01\0\0\0\0\0", Some("so"));
        assert_eq!(r.kind, "other");
    }

    #[test]
    fn text_in_every_encoding() {
        let r = p(b"fn main() {}\n", Some("rs"));
        assert_eq!(r.kind, "text");
        assert_eq!(r.text.unwrap().encoding, "utf-8");
        // CP1252 (an e-acute as 0xE9) is text, not binary.
        assert_eq!(p(b"name=caf\xe9\r\n", Some("ini")).text.unwrap().encoding, "legacy");
        assert_eq!(p(b"\xef\xbb\xbfhello", None).text.unwrap().bom, "utf-8");
        assert_eq!(p(b"\xff\xfeh\0i\0", None).text.unwrap().encoding, "utf-16le");
        assert_eq!(p(b"\xfe\xff\0h\0i", None).text.unwrap().encoding, "utf-16be");
        // UTF-16 with no BOM, by where the NULs fall.
        let le: Vec<u8> = "hello world, this is text".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(p(&le, Some("txt")).text.unwrap().encoding, "utf-16le");
        let be: Vec<u8> = "hello world, this is text".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(p(&be, Some("txt")).text.unwrap().encoding, "utf-16be");
        // An emoji cut at the end of the window is still UTF-8.
        let mut cut = b"ok ".to_vec();
        cut.extend_from_slice(&"\u{1F600}".as_bytes()[..2]);
        assert_eq!(p(&cut, None).text.unwrap().encoding, "utf-8");
        // An empty file is text.
        assert_eq!(p(b"", None).kind, "text");
    }

    #[test]
    fn a_text_extension_does_not_make_binary_text() {
        let mut head = b"looks like text at first ".repeat(400);
        head[8000] = 0;
        let r = p(&head, Some("txt"));
        assert_eq!(r.kind, "other");
        assert_eq!(r.binary_reason, Some("nul"));
    }

    #[test]
    fn a_media_extension_does_not_make_text_media() {
        // `.ts` is also MPEG transport stream; TypeScript is text.
        assert_eq!(p(b"export const x = 1;\n", Some("ts")).kind, "text");
        assert_eq!(p(b"just words", Some("mp3")).kind, "text");
    }

    #[test]
    fn control_bytes_mark_binary() {
        let r = p(b"\x01\x02\x03\x04abcdefgh", None);
        assert_eq!(r.kind, "other");
        assert_eq!(r.binary_reason, Some("control_bytes"));
        // An ANSI-coloured log (ESC is allowed) and one stray bell stay text.
        assert_eq!(p(b"\x1b[31mred\x1b[0m line\n".repeat(20).as_slice(), Some("log")).kind, "text");
        assert_eq!(p(&[b"a".repeat(500).as_slice(), b"\x07"].concat(), None).kind, "text");
    }
}
