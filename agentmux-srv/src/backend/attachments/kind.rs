// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What kind of file an attachment is, decided by content first and the
//! name's extension second (a `.txt` that is really a ZIP is not text).
//! docs/specs/SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5.

use std::io::Read;
use std::path::Path;

use image::ImageFormat;

use super::process::{self, Sniffed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// Decodable image: the image pipeline (thumbnail + send-copy).
    Image(ImageFormat),
    Svg,
    Text,
    Pdf,
    Word,
    Excel,
    PowerPoint,
    Archive,
    Audio,
    Video,
    /// An image format we can't decode (HEIC, AVIF): attached as a file.
    ImageFile,
    Other,
}

impl FileKind {
    /// The `kind` string the frontend picks tiles by.
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Image(_) => "image",
            FileKind::Svg => "svg",
            FileKind::Text => "text",
            FileKind::Pdf => "pdf",
            FileKind::Word => "word",
            FileKind::Excel => "excel",
            FileKind::PowerPoint => "powerpoint",
            FileKind::Archive => "archive",
            FileKind::Audio => "audio",
            FileKind::Video => "video",
            FileKind::ImageFile => "image_file",
            FileKind::Other => "other",
        }
    }
}

/// Extensions read as text when the content is also text.
const TEXT_EXTS: &[&str] = &[
    "txt",
    "text",
    "md",
    "markdown",
    "mdx",
    "rst",
    "adoc",
    "csv",
    "tsv",
    "json",
    "jsonl",
    "ndjson",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "properties",
    "xml",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "log",
    "rs",
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "py",
    "pyi",
    "go",
    "java",
    "kt",
    "kts",
    "scala",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "cs",
    "fs",
    "rb",
    "php",
    "swift",
    "m",
    "mm",
    "dart",
    "lua",
    "pl",
    "r",
    "jl",
    "ex",
    "exs",
    "erl",
    "hs",
    "clj",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "psm1",
    "bat",
    "cmd",
    "sql",
    "graphql",
    "gql",
    "proto",
    "vue",
    "svelte",
    "astro",
    "tf",
    "hcl",
    "gradle",
    "cmake",
    "make",
    "mk",
    "dockerfile",
    "gitignore",
    "diff",
    "patch",
    "tex",
    "bib",
    "srt",
    "vtt",
    "ipynb",
];
const AUDIO_EXTS: &[&str] = &[
    "mp3", "wav", "flac", "m4a", "aac", "ogg", "oga", "opus", "wma", "aiff", "aif",
];
const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "flv", "mpg", "mpeg",
];
const ARCHIVE_EXTS: &[&str] = &["zip", "gz", "tgz", "tar", "7z", "rar", "bz2", "xz", "zst"];

/// The file's own extension, lowercased, if it's a short plain one.
pub fn extension_of(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    (!ext.is_empty() && ext.len() <= 10 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(ext)
}

fn read_head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut head = Vec::with_capacity(n);
    (&mut f).take(n as u64).read_to_end(&mut head)?;
    Ok(head)
}

/// Text if the start is valid UTF-8 (allowing a code point cut at the end)
/// with no NUL bytes.
fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // Cut mid-character at the end of the sample is still text.
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > head.len(),
    }
}

/// Which OOXML/ODF document a ZIP holds, from its entry names or the ODF
/// `mimetype` entry. `None` for an ordinary archive.
fn office_kind_of_zip(path: &Path) -> Option<FileKind> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    if zip.len() > super::extract::MAX_ZIP_ENTRIES {
        return None;
    }
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("word/document.xml") {
        return Some(FileKind::Word);
    }
    if has("xl/workbook.xml") || has("xl/workbook.bin") {
        return Some(FileKind::Excel);
    }
    if has("ppt/presentation.xml") {
        return Some(FileKind::PowerPoint);
    }
    if has("mimetype") {
        let mut mt = String::new();
        if let Ok(e) = zip.by_name("mimetype") {
            let _ = e.take(200).read_to_string(&mut mt);
        }
        return match mt.trim() {
            "application/vnd.oasis.opendocument.text" => Some(FileKind::Word),
            "application/vnd.oasis.opendocument.spreadsheet" => Some(FileKind::Excel),
            "application/vnd.oasis.opendocument.presentation" => Some(FileKind::PowerPoint),
            _ => None,
        };
    }
    None
}

/// Classify the file at `path`, whose display name is `name`.
pub fn classify(path: &Path, name: &str) -> std::io::Result<FileKind> {
    let head = read_head(path, 8192)?;
    let ext = extension_of(name);
    let ext = ext.as_deref();
    Ok(classify_bytes(&head, ext, || office_kind_of_zip(path)))
}

/// [`classify`] on the first bytes, with the ZIP inspection injected (tests).
pub fn classify_bytes(
    head: &[u8],
    ext: Option<&str>,
    zip_kind: impl FnOnce() -> Option<FileKind>,
) -> FileKind {
    match process::sniff(head) {
        Sniffed::Image(f) => return FileKind::Image(f),
        Sniffed::Heic | Sniffed::Avif => return FileKind::ImageFile,
        Sniffed::Svg => return FileKind::Svg,
        Sniffed::NotImage => {}
    }
    if head.starts_with(b"%PDF-") {
        return FileKind::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return zip_kind().unwrap_or(FileKind::Archive);
    }
    // OLE2 compound file: legacy Office. The extension says which.
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return match ext {
            Some("doc") | Some("dot") => FileKind::Word,
            Some("xls") | Some("xlt") => FileKind::Excel,
            Some("ppt") | Some("pps") | Some("pot") => FileKind::PowerPoint,
            _ => FileKind::Other,
        };
    }
    if head.starts_with(b"{\\rtf") {
        return FileKind::Word;
    }
    if head.starts_with(&[0x1F, 0x8B])
        || head.starts_with(b"7z\xBC\xAF\x27\x1C")
        || head.starts_with(b"Rar!")
        || head.starts_with(b"BZh")
        || head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00])
        || head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
        || (head.len() > 262 && &head[257..262] == b"ustar")
    {
        return FileKind::Archive;
    }
    if head.starts_with(b"ID3")
        || head.starts_with(b"fLaC")
        || head.starts_with(b"OggS")
        || (head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WAVE")
    {
        return FileKind::Audio;
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
        || (head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"AVI ")
    {
        return FileKind::Video;
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        // MP4-family container: audio or video by extension.
        return if ext.is_some_and(|e| AUDIO_EXTS.contains(&e)) {
            FileKind::Audio
        } else {
            FileKind::Video
        };
    }
    let text = looks_like_text(head);
    match ext {
        Some(e) if text && (TEXT_EXTS.contains(&e)) => FileKind::Text,
        Some(e) if AUDIO_EXTS.contains(&e) => FileKind::Audio,
        Some(e) if VIDEO_EXTS.contains(&e) => FileKind::Video,
        Some(e) if ARCHIVE_EXTS.contains(&e) => FileKind::Archive,
        // An unknown or missing extension on text content is still text;
        // a known-binary extension on text-looking bytes (rare) stays Other.
        _ if text && !head.is_empty() => FileKind::Text,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(head: &[u8], ext: Option<&str>) -> FileKind {
        classify_bytes(head, ext, || None)
    }

    #[test]
    fn content_decides_before_the_name() {
        assert_eq!(kind(b"%PDF-1.7\n...", Some("txt")), FileKind::Pdf);
        assert_eq!(kind(b"PK\x03\x04rest", Some("txt")), FileKind::Archive);
        assert_eq!(
            classify_bytes(b"PK\x03\x04rest", Some("zip"), || Some(FileKind::Word)),
            FileKind::Word
        );
        assert_eq!(
            kind(&[0x89, b'P', b'N', b'G', 0, 0], Some("docx")),
            FileKind::Image(ImageFormat::Png)
        );
        assert_eq!(
            kind(b"\0\0\0\x18ftypheic\0\0\0\0", Some("jpg")),
            FileKind::ImageFile
        );
        assert_eq!(kind(b"<svg xmlns='x'/>", Some("svg")), FileKind::Svg);
    }

    #[test]
    fn text_needs_text_content() {
        assert_eq!(kind(b"fn main() {}\n", Some("rs")), FileKind::Text);
        assert_eq!(kind(b"a,b\n1,2\n", Some("csv")), FileKind::Text);
        assert_eq!(kind(b"just words", None), FileKind::Text);
        assert_eq!(
            kind(b"MZ\x90\0\x03\0\0\0", Some("txt")),
            FileKind::Other,
            "an exe renamed .txt"
        );
        // An empty .txt is text; an empty file with no name hint is not.
        assert_eq!(kind(b"", Some("txt")), FileKind::Text);
        assert_eq!(kind(b"", None), FileKind::Other);
    }

    #[test]
    fn an_office_extension_alone_never_makes_an_office_file() {
        // A Windows executable and an ELF binary renamed as documents.
        assert_eq!(
            kind(b"MZ\x90\0\x03\0\0\0\x04\0", Some("doc")),
            FileKind::Other
        );
        assert_eq!(kind(b"\x7fELF\x02\x01\x01\0", Some("xls")), FileKind::Other);
        assert_eq!(kind(b"MZ\x90\0\x03\0\0\0", Some("rtf")), FileKind::Other);
    }

    #[test]
    fn legacy_office_goes_by_extension() {
        let ole = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0];
        assert_eq!(kind(&ole, Some("doc")), FileKind::Word);
        assert_eq!(kind(&ole, Some("xls")), FileKind::Excel);
        assert_eq!(kind(&ole, Some("ppt")), FileKind::PowerPoint);
        assert_eq!(kind(&ole, Some("msi")), FileKind::Other);
        assert_eq!(kind(b"{\\rtf1\\ansi", Some("rtf")), FileKind::Word);
    }

    #[test]
    fn media_and_archives() {
        assert_eq!(kind(b"ID3\x04\0\0", Some("mp3")), FileKind::Audio);
        assert_eq!(
            kind(b"\0\0\0\x20ftypisom\0\0", Some("mp4")),
            FileKind::Video
        );
        assert_eq!(
            kind(b"\0\0\0\x20ftypM4A \0\0", Some("m4a")),
            FileKind::Audio
        );
        assert_eq!(kind(&[0x1F, 0x8B, 8, 0], Some("gz")), FileKind::Archive);
        assert_eq!(kind(b"7z\xBC\xAF\x27\x1C\0", None), FileKind::Archive);
    }

    #[test]
    fn extension_is_sanitized() {
        assert_eq!(extension_of("Report.DOCX").as_deref(), Some("docx"));
        assert_eq!(extension_of("noext"), None);
        assert_eq!(extension_of("weird.e x e"), None);
    }
}
