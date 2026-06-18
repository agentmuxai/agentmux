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
            // allow_utf8=true: detector may still pick UTF-8 for ASCII-heavy input.
            (det.guess(None, true), "none", bytes)
        };

    let (cow, had_decode_errors) = enc.decode_without_bom_handling(body);
    let content = cow.into_owned();
    let line_ending = if content.contains("\r\n") { "crlf" } else { "lf" };

    DecodedFile {
        content,
        encoding: enc.name().to_string(),
        bom,
        line_ending,
        had_decode_errors,
    }
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

    let enc = if encoding_label.is_empty() {
        UTF_8
    } else {
        Encoding::for_label(encoding_label.as_bytes()).unwrap_or(UTF_8)
    };

    // encoding_rs has no UTF-16 *encoder* (per the Encoding Standard its
    // `encode()` emits UTF-8). Round-trip UTF-16 by hand so a UTF-16 file saves
    // back as UTF-16, not UTF-8.
    let (mut out, had_unmappable): (Vec<u8>, bool) = if enc == UTF_16LE {
        let mut v = Vec::with_capacity(normalized.len() * 2);
        for u in normalized.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        (v, false)
    } else if enc == UTF_16BE {
        let mut v = Vec::with_capacity(normalized.len() * 2);
        for u in normalized.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        (v, false)
    } else {
        let (bytes, _enc, had_unmappable) = enc.encode(&normalized);
        (bytes.into_owned(), had_unmappable)
    };

    // Re-emit a BOM if the file had/uses one.
    let bom_bytes: &[u8] = match bom {
        "utf-8" => b"\xEF\xBB\xBF",
        "utf-16le" => b"\xFF\xFE",
        "utf-16be" => b"\xFE\xFF",
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
}
