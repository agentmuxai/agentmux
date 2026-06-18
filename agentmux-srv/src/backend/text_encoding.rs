// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Text encoding detection + transcoding for the editor.
//!
//! The editor used to read files with `std::fs::read_to_string`, which rejects
//! anything that isn't valid UTF-8 — so a Windows-1252 `.ini`, a UTF-16 file,
//! Shift_JIS, etc. simply failed to open. This module reads bytes, detects the
//! encoding (BOM → valid-UTF-8 → `chardetng` heuristic), and decodes to a Rust
//! `String` for the editor; and re-encodes on save so files round-trip in their
//! original encoding + BOM + line endings.
//!
//! Spec: docs/specs/SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};

/// Result of decoding a file's bytes for the editor.
pub struct DecodedFile {
    /// UTF-8 text for the editor/wire (BOM stripped).
    pub content: String,
    /// Encoding label (WHATWG / `encoding_rs` name, e.g. "windows-1252").
    pub encoding: String,
    /// Byte-order-mark that was present: "none" | "utf-8" | "utf-16le" | "utf-16be".
    pub bom: &'static str,
    /// Detected line ending: "crlf" if any CRLF present, else "lf".
    pub line_ending: &'static str,
    /// True when decoding inserted U+FFFD replacements (likely wrong encoding).
    pub had_decode_errors: bool,
}

/// Detect the encoding of `bytes` and decode to UTF-8.
///
/// Order: BOM (authoritative) → valid UTF-8 (keeps the common case exact) →
/// `chardetng` heuristic. encoding_rs always succeeds (replacement on malformed
/// input), with `had_decode_errors` flagging low-confidence results.
pub fn decode_file(bytes: &[u8]) -> DecodedFile {
    // UTF-32 BOMs must be checked BEFORE UTF-16: a UTF-32LE BOM (FF FE 00 00)
    // starts with the UTF-16LE BOM (FF FE) and would otherwise be misdecoded.
    // encoding_rs has no UTF-32 codec, so we handle it by hand.
    if bytes.starts_with(b"\xFF\xFE\x00\x00") {
        return decode_utf32(&bytes[4..], false, "utf-32le", "UTF-32LE");
    }
    if bytes.starts_with(b"\x00\x00\xFE\xFF") {
        return decode_utf32(&bytes[4..], true, "utf-32be", "UTF-32BE");
    }

    let (enc, bom, body): (&'static Encoding, &'static str, &[u8]) =
        if bytes.starts_with(b"\xEF\xBB\xBF") {
            (UTF_8, "utf-8", &bytes[3..])
        } else if bytes.starts_with(b"\xFF\xFE") {
            (UTF_16LE, "utf-16le", &bytes[2..])
        } else if bytes.starts_with(b"\xFE\xFF") {
            (UTF_16BE, "utf-16be", &bytes[2..])
        } else if std::str::from_utf8(bytes).is_ok() {
            (UTF_8, "none", bytes)
        } else {
            let mut det = chardetng::EncodingDetector::new();
            det.feed(bytes, true);
            // guess(tld, allow_eu): allow_eu=false — don't bias toward
            // Central-European windows-1250 (no locale signal here; biasing
            // would misread ambiguous Western text). tld=None: no domain hint.
            (det.guess(None, false), "none", bytes)
        };

    let (cow, had_decode_errors) = enc.decode_without_bom_handling(body);
    let content = cow.into_owned();
    let line_ending = detect_line_ending(&content);

    DecodedFile {
        content,
        encoding: enc.name().to_string(),
        bom,
        line_ending,
        had_decode_errors,
    }
}

/// The file's line ending, from the **first** line break (what VS Code does).
/// Using "any CRLF present" would flag a mostly-LF file as CRLF and convert all
/// its LF lines on save.
fn detect_line_ending(s: &str) -> &'static str {
    match s.find('\n') {
        Some(i) if i > 0 && s.as_bytes()[i - 1] == b'\r' => "crlf",
        _ => "lf",
    }
}

/// Decode UTF-32 (LE/BE) bytes by hand — encoding_rs has no UTF-32 codec.
fn decode_utf32(body: &[u8], big_endian: bool, bom: &'static str, label: &'static str) -> DecodedFile {
    let mut content = String::new();
    let mut had_decode_errors = false;
    for chunk in body.chunks(4) {
        if chunk.len() < 4 {
            had_decode_errors = true; // trailing partial code unit
            break;
        }
        let cp = if big_endian {
            u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        } else {
            u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        };
        match char::from_u32(cp) {
            Some(c) => content.push(c),
            None => {
                content.push('\u{FFFD}');
                had_decode_errors = true;
            }
        }
    }
    let line_ending = detect_line_ending(&content);
    DecodedFile {
        content,
        encoding: label.to_string(),
        bom,
        line_ending,
        had_decode_errors,
    }
}

fn utf16_bytes(s: &str, big_endian: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        v.extend_from_slice(&if big_endian { u.to_be_bytes() } else { u.to_le_bytes() });
    }
    v
}

fn utf32_bytes(s: &str, big_endian: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.chars().count() * 4);
    for c in s.chars() {
        let u = c as u32;
        v.extend_from_slice(&if big_endian { u.to_be_bytes() } else { u.to_le_bytes() });
    }
    v
}

/// Encode editor text (`\n`-delimited UTF-8) back to bytes in `encoding_label`,
/// applying `bom` and `line_ending`. Returns the bytes to write plus whether any
/// character was unmappable in the target encoding (the caller may warn).
///
/// `encoding_label` falls back to UTF-8 when unknown/empty, so existing callers
/// that don't pass an encoding keep writing UTF-8.
pub fn encode_file(
    content: &str,
    encoding_label: &str,
    bom: &str,
    line_ending: &str,
) -> (Vec<u8>, bool) {
    // Normalize to the file's line-ending convention. The editor buffer uses
    // `\n`; collapse any stray CRLF first so we don't double-convert.
    let normalized = if line_ending == "crlf" {
        content.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        content.to_string()
    };

    // UTF-16/UTF-32 are encoded by hand: encoding_rs has no UTF-16/32 *encoder*
    // (per the Encoding Standard its `encode()` emits UTF-8), so a UTF-16/32
    // file would otherwise save back as UTF-8. Match on the label first; the
    // encoding_rs fallback handles legacy single-byte/CJK encodings, where
    // `had_unmappable` reports characters outside the target charset.
    let (mut out, had_unmappable): (Vec<u8>, bool) = match encoding_label.to_ascii_lowercase().as_str() {
        "utf-16le" => (utf16_bytes(&normalized, false), false),
        "utf-16be" => (utf16_bytes(&normalized, true), false),
        "utf-32le" => (utf32_bytes(&normalized, false), false),
        "utf-32be" => (utf32_bytes(&normalized, true), false),
        other => {
            let enc = if other.is_empty() {
                UTF_8
            } else {
                Encoding::for_label(other.as_bytes()).unwrap_or(UTF_8)
            };
            let (bytes, _enc, had_unmappable) = enc.encode(&normalized);
            (bytes.into_owned(), had_unmappable)
        }
    };

    // Re-emit a BOM if the file had/uses one.
    let bom_bytes: &[u8] = match bom {
        "utf-8" => b"\xEF\xBB\xBF",
        "utf-16le" => b"\xFF\xFE",
        "utf-16be" => b"\xFE\xFF",
        "utf-32le" => b"\xFF\xFE\x00\x00",
        "utf-32be" => b"\x00\x00\xFE\xFF",
        _ => b"",
    };
    if !bom_bytes.is_empty() {
        let mut prefixed = Vec::with_capacity(bom_bytes.len() + out.len());
        prefixed.extend_from_slice(bom_bytes);
        prefixed.append(&mut out);
        out = prefixed;
    }

    (out, had_unmappable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_is_utf8() {
        let d = decode_file(b"hello = world\n");
        assert_eq!(d.content, "hello = world\n");
        assert_eq!(d.encoding, "UTF-8");
        assert_eq!(d.bom, "none");
        assert_eq!(d.line_ending, "lf");
        assert!(!d.had_decode_errors);
    }

    #[test]
    fn windows_1252_ini_decodes() {
        // 0x92 = right single quote, 0xE9 = é in Windows-1252 — invalid UTF-8.
        let bytes = b"name = O\x92Brien caf\xE9\r\n";
        assert!(std::str::from_utf8(bytes).is_err(), "precondition: not UTF-8");
        let d = decode_file(bytes);
        assert!(d.content.contains("O\u{2019}Brien"), "got {:?}", d.content);
        assert!(d.content.contains("caf\u{e9}"));
        assert_eq!(d.line_ending, "crlf");
        assert_ne!(d.encoding, "UTF-8"); // heuristic picked a legacy encoding
    }

    #[test]
    fn utf8_bom_is_stripped_and_remembered() {
        let d = decode_file(b"\xEF\xBB\xBFhi");
        assert_eq!(d.content, "hi");
        assert_eq!(d.bom, "utf-8");
        assert_eq!(d.encoding, "UTF-8");
    }

    #[test]
    fn utf16le_bom_decodes() {
        // "Hi" in UTF-16LE with BOM.
        let d = decode_file(b"\xFF\xFEH\x00i\x00");
        assert_eq!(d.content, "Hi");
        assert_eq!(d.bom, "utf-16le");
    }

    #[test]
    fn windows_1252_round_trips() {
        let original = "café — O\u{2019}Brien";
        let (bytes, unmappable) = encode_file(original, "windows-1252", "none", "lf");
        assert!(!unmappable);
        assert!(std::str::from_utf8(&bytes).is_err(), "should be legacy bytes");
        let back = decode_file(&bytes);
        // chardetng may not name it "windows-1252" exactly, but the text must round-trip.
        assert_eq!(back.content, original);
    }

    #[test]
    fn utf16le_round_trips_with_bom() {
        let (bytes, _) = encode_file("Hi", "utf-16le", "utf-16le", "lf");
        assert_eq!(&bytes[..2], b"\xFF\xFE");
        let back = decode_file(&bytes);
        assert_eq!(back.content, "Hi");
        assert_eq!(back.bom, "utf-16le");
    }

    #[test]
    fn crlf_line_ending_applied_on_write() {
        let (bytes, _) = encode_file("a\nb", "UTF-8", "none", "crlf");
        assert_eq!(bytes, b"a\r\nb");
    }

    #[test]
    fn unknown_label_falls_back_to_utf8() {
        let (bytes, _) = encode_file("hi", "definitely-not-an-encoding", "none", "lf");
        assert_eq!(bytes, b"hi");
    }

    #[test]
    fn line_ending_uses_first_break_not_any_crlf() {
        // Mostly-LF with a later CRLF → first break is LF → "lf"
        // (so we don't normalize all the LF lines to CRLF on save).
        assert_eq!(decode_file(b"a\nb\r\nc\n").line_ending, "lf");
        assert_eq!(decode_file(b"a\r\nb\nc").line_ending, "crlf");
        assert_eq!(decode_file(b"no breaks").line_ending, "lf");
    }

    #[test]
    fn utf32le_bom_decodes_not_as_utf16() {
        // "Hi" in UTF-32LE with BOM (FF FE 00 00) — must NOT be read as UTF-16LE.
        let bytes = b"\xFF\xFE\x00\x00H\x00\x00\x00i\x00\x00\x00";
        let d = decode_file(bytes);
        assert_eq!(d.content, "Hi");
        assert_eq!(d.bom, "utf-32le");
        assert_eq!(d.encoding, "UTF-32LE");
    }

    #[test]
    fn utf32_round_trips() {
        let (bytes, _) = encode_file("Hi", "utf-32le", "utf-32le", "lf");
        assert_eq!(&bytes[..4], b"\xFF\xFE\x00\x00");
        let back = decode_file(&bytes);
        assert_eq!(back.content, "Hi");
        assert_eq!(back.bom, "utf-32le");
    }

    #[test]
    fn unmappable_char_is_flagged_not_lossy() {
        // An emoji can't be represented in windows-1252 → had_unmappable=true
        // (the caller refuses the write instead of emitting NCR garbage).
        let (_bytes, had_unmappable) = encode_file("hi 😀", "windows-1252", "none", "lf");
        assert!(had_unmappable, "emoji in cp1252 must flag unmappable");
        // UTF-8 represents everything → never unmappable.
        let (_b, u8_unmappable) = encode_file("hi 😀", "UTF-8", "none", "lf");
        assert!(!u8_unmappable);
    }
}
