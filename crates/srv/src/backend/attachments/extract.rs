// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Everything derived from a non-image attachment: a short text preview for
//! the tile, a text version of Office documents (no agent reads those), the
//! page count of a PDF, and whether an Office file carries macros.
//! docs/specs/SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §6.
//!
//! Office files are ZIPs, so every read here is bounded: an entry-count cap,
//! a declared compression-ratio cap, one decompressed-bytes budget shared by
//! all parts (the zip crate never reads past an entry's declared size, so the
//! declared sizes bound the work), quick-xml with predefined entities only,
//! a cap on the text produced, and a wall-clock deadline.
//!
//! Parsing runs in a child process ([`CHILD_ARG`]): srv re-runs its own
//! executable, reads the result from its stdout and kills it after
//! [`DEADLINE`]. Whatever a hostile file does to a parser (panic, deep
//! recursion, a sheet that loads for minutes, memory) happens to the child.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use quick_xml::events::Event;
use quick_xml::Reader;

use super::kind::FileKind;

pub const MAX_ZIP_ENTRIES: usize = 10_000;
/// Total bytes all parts of one document may decompress to.
pub const MAX_DECOMPRESSED_BYTES: u64 = 200 * 1024 * 1024;
/// An entry claiming more than this ratio of uncompressed to compressed
/// size is treated as a bomb.
pub const MAX_COMPRESSION_RATIO: u64 = 100;
/// Text versions are cut here (Codex refuses prompts over ~1M characters).
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const DEADLINE: Duration = Duration::from_secs(30);
/// PDFs larger than this aren't opened for a page count.
pub const MAX_PDF_BYTES_FOR_COUNT: u64 = 100 * 1024 * 1024;

pub const PREVIEW_LINES: usize = 40;
pub const PREVIEW_BYTES: usize = 2048;

/// The first lines of a text file, cut at a UTF-8 boundary.
pub fn text_preview(path: &Path) -> std::io::Result<String> {
    let mut head = Vec::with_capacity(PREVIEW_BYTES);
    std::fs::File::open(path)?
        .take(PREVIEW_BYTES as u64)
        .read_to_end(&mut head)?;
    let text = match std::str::from_utf8(&head) {
        Ok(s) => s.to_string(),
        Err(e) => String::from_utf8_lossy(&head[..e.valid_up_to()]).into_owned(),
    };
    Ok(text
        .lines()
        .take(PREVIEW_LINES)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// What can be read out of a PDF.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PdfFacts {
    pub pages: Option<u32>,
    /// Extracted text with the AgentMux header, or `None` when the PDF has
    /// no extractable text (scanned, encrypted, malformed).
    pub text: Option<String>,
}

/// The hidden argument srv re-executes itself with to read a document in a
/// separate process: `srv __extract <pdf|text> <kind> <path> <name>` (see
/// [`run_child`] and `main.rs`).
pub const CHILD_ARG: &str = "__extract";

/// Page count and text of a PDF, in this process. PDF parsers can panic or
/// recurse deeply on malformed input: production code calls
/// [`pdf_facts_isolated`], which runs this in the child process.
pub fn pdf_facts_in_process(path: &Path, name: &str) -> PdfFacts {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_PDF_BYTES_FOR_COUNT {
        return PdfFacts::default();
    }
    let path = path.to_path_buf();
    let name = name.to_string();
    std::panic::catch_unwind(move || {
        let Ok(doc) = lopdf::Document::load(&path) else { return PdfFacts::default() };
        let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
        let count = (!pages.is_empty()).then_some(pages.len() as u32);
        let text = doc
            .extract_text(&pages)
            .ok()
            .filter(|t| t.chars().any(|c| !c.is_whitespace()))
            .map(|t| {
                cap_text(format!(
                    "Text extracted from {name} by AgentMux. Layout, images and scanned pages are not included.\n\n{}\n",
                    t.trim()
                ))
            });
        PdfFacts { pages: count, text }
    })
    .unwrap_or_default()
}

/// How a child run ended.
#[derive(Debug, PartialEq, Eq)]
enum ChildRun {
    Output(Vec<u8>),
    TimedOut,
    Failed,
}

/// Run `cmd` with stdout piped, killing it after `deadline`.
fn run_with_deadline(mut cmd: std::process::Command, deadline: Duration) -> ChildRun {
    use std::process::Stdio;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        cmd.no_window();
    }
    let Ok(mut child) = cmd.spawn() else {
        return ChildRun::Failed;
    };
    // Read stdout on a thread so a large text can't fill the pipe and stall
    // the child while we wait on it.
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s
                .take((MAX_TEXT_BYTES as u64) * 2 + 4096)
                .read_to_end(&mut out);
        }
        out
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return ChildRun::Failed,
            Ok(None) if started.elapsed() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return ChildRun::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return ChildRun::Failed,
        }
    }
    ChildRun::Output(reader.join().unwrap_or_default())
}

/// `srv __extract <mode> <kind> <path> <name>` with the deadline.
#[cfg(not(test))]
fn run_isolated(mode: &str, kind: FileKind, path: &Path, name: &str) -> ChildRun {
    let Ok(exe) = std::env::current_exe() else {
        return ChildRun::Failed;
    };
    let mut cmd = std::process::Command::new(exe);
    // It only reads one file: none of this instance's identity goes with it.
    crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
    cmd.arg(CHILD_ARG)
        .arg(mode)
        .arg(kind.as_str())
        .arg(path)
        .arg(name);
    run_with_deadline(cmd, DEADLINE)
}

/// Under `cargo test` the current executable is the test harness, which
/// would read `__extract …` as test filters: run the child's code in-process.
#[cfg(test)]
fn run_isolated(mode: &str, kind: FileKind, path: &Path, name: &str) -> ChildRun {
    match child_output(mode, kind.as_str(), path, name) {
        Some(json) => ChildRun::Output(json.into_bytes()),
        None => ChildRun::Failed,
    }
}

/// [`pdf_facts_in_process`] in the child process. A crash or timeout just
/// means no facts.
pub fn pdf_facts_isolated(path: &Path, name: &str) -> PdfFacts {
    match run_isolated("pdf", FileKind::Pdf, path, name) {
        ChildRun::Output(out) => serde_json::from_slice(&out).unwrap_or_default(),
        _ => PdfFacts::default(),
    }
}

/// What the child reports for a text version: the text, none for this
/// kind, or why it failed.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TextOutcome {
    text: Option<String>,
    error: Option<String>,
}

/// [`document_text`] in the child process.
pub fn document_text_isolated(
    path: &Path,
    kind: FileKind,
    name: &str,
) -> Result<Option<String>, ExtractError> {
    match run_isolated("text", kind, path, name) {
        ChildRun::Output(out) => match serde_json::from_slice::<TextOutcome>(&out) {
            Ok(TextOutcome { error: Some(e), .. }) => Err(err(e)),
            Ok(TextOutcome { text, .. }) => Ok(text),
            Err(_) => Err(err("the document couldn't be read")),
        },
        ChildRun::TimedOut => Err(err("extraction took too long")),
        ChildRun::Failed => Err(err("the document couldn't be read")),
    }
}

/// Text version of a Word/Excel/PowerPoint/RTF document, in this process.
/// Production code calls [`document_text_isolated`].
pub fn document_text(
    path: &Path,
    kind: FileKind,
    name: &str,
) -> Result<Option<String>, ExtractError> {
    let is_rtf = std::fs::File::open(path)
        .and_then(|mut f| {
            let mut head = [0u8; 5];
            f.read_exact(&mut head).map(|_| &head == b"{\\rtf")
        })
        .unwrap_or(false);
    if is_rtf {
        rtf_text(path, name).map(Some)
    } else {
        office_text(path, kind, name)
    }
}

fn kind_from_str(s: &str) -> Option<FileKind> {
    Some(match s {
        "pdf" => FileKind::Pdf,
        "word" => FileKind::Word,
        "excel" => FileKind::Excel,
        "powerpoint" => FileKind::PowerPoint,
        _ => return None,
    })
}

/// Entry point for the child process: `srv __extract <mode> <kind> <path>
/// <name>`. Prints [`PdfFacts`] (`pdf`) or a text outcome (`text`) as JSON.
pub fn run_child(args: &[String]) -> i32 {
    let (Some(mode), Some(kind), Some(path), Some(name)) =
        (args.first(), args.get(1), args.get(2), args.get(3))
    else {
        return 2;
    };
    match child_output(mode, kind, Path::new(path), name) {
        Some(json) => {
            println!("{json}");
            0
        }
        None => 2,
    }
}

/// What the child prints for one request; `None` for a malformed request.
fn child_output(mode: &str, kind: &str, path: &Path, name: &str) -> Option<String> {
    let json = match (mode, kind_from_str(kind)) {
        ("pdf", _) => serde_json::to_string(&pdf_facts_in_process(path, name)),
        ("text", Some(kind)) => {
            let outcome = match std::panic::catch_unwind(|| document_text(path, kind, name)) {
                Ok(Ok(text)) => TextOutcome { text, error: None },
                Ok(Err(e)) => TextOutcome {
                    text: None,
                    error: Some(e.0),
                },
                Err(_) => TextOutcome {
                    text: None,
                    error: Some("the document couldn't be read".into()),
                },
            };
            serde_json::to_string(&outcome)
        }
        _ => return None,
    };
    Some(json.unwrap_or_else(|_| "{}".into()))
}

/// Plain text of an RTF document: control words dropped, `{\*…}` and a few
/// non-text destinations skipped, `\par`/`\line`/`\tab`, `\'hh` and `\uN`
/// decoded.
pub fn rtf_text(path: &Path, name: &str) -> Result<String, ExtractError> {
    let mut raw = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| err(e.to_string()))?
        .take(MAX_DECOMPRESSED_BYTES)
        .read_to_end(&mut raw)
        .map_err(|e| err(e.to_string()))?;
    let deadline = Instant::now() + DEADLINE;
    // RTF is 7-bit: anything else arrives as `\'hh` or `\uN`. A stray high
    // byte reads as Latin-1, like `\'hh` does. Byte-wise, so a big file
    // costs its own size and nothing more.
    let chars: &[u8] = &raw;
    let mut out = String::new();
    // Group depths at which a skipped destination started.
    let mut skip_from: Option<usize> = None;
    let mut depth = 0usize;
    let mut uc_skip = 0usize;
    let mut i = 0;
    while i < chars.len() {
        if i % 4096 == 0 && Instant::now() > deadline {
            return Err(err("extraction took too long"));
        }
        let c = chars[i];
        match c {
            b'{' => {
                depth += 1;
                i += 1;
                // `{\*\dest …}` and known non-text destinations are skipped whole.
                let rest = &chars[i..chars.len().min(i + 12)];
                if skip_from.is_none()
                    && [
                        "\\*",
                        "\\fonttbl",
                        "\\colortbl",
                        "\\stylesheet",
                        "\\info",
                        "\\pict",
                        "\\header",
                        "\\footer",
                    ]
                    .iter()
                    .any(|d| rest.starts_with(d.as_bytes()))
                {
                    skip_from = Some(depth);
                }
            }
            b'}' => {
                if skip_from == Some(depth) {
                    skip_from = None;
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&n) = chars.get(i) else { break };
                if n == b'\'' {
                    let hex = chars
                        .get(i + 1..i + 3)
                        .and_then(|s| std::str::from_utf8(s).ok())
                        .unwrap_or_default();
                    if skip_from.is_none() {
                        if let Ok(b) = u8::from_str_radix(hex, 16) {
                            if uc_skip > 0 {
                                uc_skip -= 1;
                            } else {
                                out.push(b as char);
                            }
                        }
                    }
                    i += 3;
                    continue;
                }
                if !n.is_ascii_alphabetic() {
                    // `\\`, `\{`, `\}` are literals; other control symbols drop.
                    if skip_from.is_none() && matches!(n, b'\\' | b'{' | b'}') {
                        out.push(n as char);
                    }
                    i += 1;
                    continue;
                }
                let start = i;
                while chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                    i += 1;
                }
                let word = std::str::from_utf8(&chars[start..i]).unwrap_or_default();
                let num_start = i;
                if chars.get(i) == Some(&b'-') {
                    i += 1;
                }
                while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    i += 1;
                }
                let num: Option<i32> = std::str::from_utf8(&chars[num_start..i])
                    .ok()
                    .and_then(|n| n.parse().ok());
                if chars.get(i) == Some(&b' ') {
                    i += 1;
                }
                if skip_from.is_some() {
                    continue;
                }
                match word {
                    "par" | "line" | "sect" | "page" => out.push('\n'),
                    "tab" => out.push('\t'),
                    "u" => {
                        if let Some(n) = num {
                            let code = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                            if let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                            // The ANSI fallback character that follows.
                            uc_skip = 1;
                        }
                    }
                    _ => {}
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                if skip_from.is_none() && depth > 0 {
                    if uc_skip > 0 {
                        uc_skip -= 1;
                    } else {
                        out.push(c as char);
                    }
                }
                i += 1;
            }
        }
        if out.len() > MAX_TEXT_BYTES {
            break;
        }
    }
    Ok(cap_text(format!(
        "Text extracted from {name} by AgentMux. Formatting, images and embedded objects are not included.\n\n{}\n",
        out.trim()
    )))
}

/// Why no text version was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractError(pub String);

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: impl Into<String>) -> ExtractError {
    ExtractError(msg.into())
}

/// Text version of an Office document (`kind` Word/Excel/PowerPoint), with
/// a header saying where it came from. `Ok(None)` for kinds with no text
/// version (legacy binary .doc/.ppt).
pub fn office_text(
    path: &Path,
    kind: FileKind,
    name: &str,
) -> Result<Option<String>, ExtractError> {
    let deadline = Instant::now() + DEADLINE;
    let body = match kind {
        FileKind::Excel => Some(spreadsheet_text(path, deadline)?),
        FileKind::Word | FileKind::PowerPoint => {
            let Some(mut zip) = open_checked_zip(path)? else {
                return Ok(None);
            };
            let mut budget = MAX_DECOMPRESSED_BYTES;
            let names: Vec<String> = zip.file_names().map(str::to_string).collect();
            let text = if names.iter().any(|n| n == "word/document.xml") {
                docx_text(&mut zip, &names, &mut budget, deadline)?
            } else if names.iter().any(|n| n == "ppt/presentation.xml") {
                pptx_text(&mut zip, &mut budget, deadline)?
            } else if names.iter().any(|n| n == "content.xml") {
                // ODF text or presentation: paragraphs in content.xml.
                let xml = read_part(&mut zip, "content.xml", &mut budget)?;
                xml_text(&xml, OdfOrOoxml::Odf, deadline)?
            } else {
                return Ok(None);
            };
            Some(text)
        }
        _ => None,
    };
    Ok(body.map(|b| {
        let mut out = format!(
            "Text extracted from {name} by AgentMux. Formatting, images and embedded objects are not included.\n\n"
        );
        out.push_str(b.trim());
        out.push('\n');
        cap_text(out)
    }))
}

/// Whether an OOXML package carries a VBA project, whatever its extension.
pub fn has_macros(path: &Path) -> bool {
    let Ok(Some(mut zip)) = open_checked_zip(path) else {
        return false;
    };
    if zip
        .file_names()
        .any(|n| n.to_ascii_lowercase().contains("vbaproject"))
    {
        return true;
    }
    let mut budget = 1024 * 1024;
    read_part(&mut zip, "[Content_Types].xml", &mut budget)
        .map(|ct| {
            let ct = ct.to_ascii_lowercase();
            ct.contains("vbaproject") || ct.contains("macroenabled")
        })
        .unwrap_or(false)
}

fn cap_text(mut s: String) -> String {
    if s.len() <= MAX_TEXT_BYTES {
        return s;
    }
    let mut cut = MAX_TEXT_BYTES;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push_str("\n\n[truncated: the text version is limited to 2 MB]\n");
    s
}

/// Open a ZIP after checking entry count and every entry's declared ratio.
/// `Ok(None)` if it isn't a ZIP at all.
fn open_checked_zip(path: &Path) -> Result<Option<zip::ZipArchive<std::fs::File>>, ExtractError> {
    let f = std::fs::File::open(path).map_err(|e| err(e.to_string()))?;
    let Ok(mut zip) = zip::ZipArchive::new(f) else {
        return Ok(None);
    };
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(err(format!(
            "the document has too many parts ({})",
            zip.len()
        )));
    }
    let mut declared: u64 = 0;
    for i in 0..zip.len() {
        let e = zip.by_index_raw(i).map_err(|e| err(e.to_string()))?;
        let (size, packed) = (e.size(), e.compressed_size().max(1));
        if size / packed > MAX_COMPRESSION_RATIO && size > 1024 * 1024 {
            return Err(err(
                "the document is compressed suspiciously well (possible zip bomb)",
            ));
        }
        declared = declared.saturating_add(size);
    }
    if declared > MAX_DECOMPRESSED_BYTES * 4 {
        return Err(err("the document is too large once decompressed"));
    }
    Ok(Some(zip))
}

/// Read one part as UTF-8, charging its bytes to `budget`.
fn read_part(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    budget: &mut u64,
) -> Result<String, ExtractError> {
    let entry = zip.by_name(name).map_err(|e| err(format!("{name}: {e}")))?;
    let mut buf = Vec::new();
    entry
        .take(*budget + 1)
        .read_to_end(&mut buf)
        .map_err(|e| err(format!("{name}: {e}")))?;
    if buf.len() as u64 > *budget {
        return Err(err("the document is too large once decompressed"));
    }
    *budget -= buf.len() as u64;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[derive(Clone, Copy)]
enum OdfOrOoxml {
    /// `<w:t>` / `<a:t>` runs, `<w:p>` / `<a:p>` paragraphs.
    Ooxml,
    /// `<text:p>` / `<text:h>` paragraphs holding text directly.
    Odf,
}

/// Plain text of one WordprocessingML, DrawingML or ODF part.
fn xml_text(xml: &str, dialect: OdfOrOoxml, deadline: Instant) -> Result<String, ExtractError> {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut in_run = false;
    let mut para_depth = 0usize;
    loop {
        if Instant::now() > deadline {
            return Err(err("extraction took too long"));
        }
        let ev = reader
            .read_event()
            .map_err(|e| err(format!("unreadable XML: {e}")))?;
        match ev {
            Event::Start(e) => match (dialect, e.local_name().as_ref()) {
                (OdfOrOoxml::Ooxml, b"t") => in_run = true,
                (OdfOrOoxml::Odf, b"p" | b"h") => para_depth += 1,
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"tab" => out.push('\t'),
                b"br" | b"line-break" => out.push('\n'),
                b"s" if matches!(dialect, OdfOrOoxml::Odf) => out.push(' '),
                _ => {}
            },
            Event::End(e) => match (dialect, e.local_name().as_ref()) {
                (OdfOrOoxml::Ooxml, b"t") => in_run = false,
                (OdfOrOoxml::Ooxml, b"p") => out.push('\n'),
                (OdfOrOoxml::Odf, b"p" | b"h") => {
                    para_depth = para_depth.saturating_sub(1);
                    out.push('\n');
                }
                _ => {}
            },
            Event::Text(t) => {
                let take = match dialect {
                    OdfOrOoxml::Ooxml => in_run,
                    OdfOrOoxml::Odf => para_depth > 0,
                };
                if take {
                    out.push_str(
                        &t.decode()
                            .map_err(|e| err(format!("unreadable XML text: {e}")))?,
                    );
                }
            }
            Event::GeneralRef(r) => {
                let take = match dialect {
                    OdfOrOoxml::Ooxml => in_run,
                    OdfOrOoxml::Odf => para_depth > 0,
                };
                if take {
                    // Only the predefined entities and character references;
                    // anything else (custom entities) is dropped, never expanded.
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        out.push(c);
                    } else {
                        let name: &[u8] = &r;
                        match name {
                            b"amp" => out.push('&'),
                            b"lt" => out.push('<'),
                            b"gt" => out.push('>'),
                            b"quot" => out.push('"'),
                            b"apos" => out.push('\''),
                            _ => {}
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        if out.len() > MAX_TEXT_BYTES {
            break;
        }
    }
    Ok(out)
}

fn docx_text(
    zip: &mut zip::ZipArchive<std::fs::File>,
    names: &[String],
    budget: &mut u64,
    deadline: Instant,
) -> Result<String, ExtractError> {
    let mut out = xml_text(
        &read_part(zip, "word/document.xml", budget)?,
        OdfOrOoxml::Ooxml,
        deadline,
    )?;
    // Headers, footers and notes after the body, in name order.
    let mut extra: Vec<&String> = names
        .iter()
        .filter(|n| {
            let n = n.as_str();
            n.starts_with("word/")
                && n.ends_with(".xml")
                && (n.starts_with("word/header")
                    || n.starts_with("word/footer")
                    || n == "word/footnotes.xml"
                    || n == "word/endnotes.xml")
        })
        .collect();
    extra.sort();
    for name in extra {
        let text = xml_text(&read_part(zip, name, budget)?, OdfOrOoxml::Ooxml, deadline)?;
        if !text.trim().is_empty() {
            out.push_str("\n\n");
            out.push_str(text.trim());
        }
    }
    Ok(out)
}

/// Slides in presentation order: `presentation.xml`'s `sldIdLst` gives
/// relationship ids, `presentation.xml.rels` maps them to slide parts.
fn pptx_text(
    zip: &mut zip::ZipArchive<std::fs::File>,
    budget: &mut u64,
    deadline: Instant,
) -> Result<String, ExtractError> {
    let pres = read_part(zip, "ppt/presentation.xml", budget)?;
    let rels = read_part(zip, "ppt/_rels/presentation.xml.rels", budget).unwrap_or_default();
    let targets = relationship_targets(&rels);
    let mut order: Vec<String> = Vec::new();
    let mut reader = Reader::from_str(&pres);
    loop {
        match reader
            .read_event()
            .map_err(|e| err(format!("unreadable XML: {e}")))?
        {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sldId" => {
                for a in e.attributes().flatten() {
                    if a.key.local_name().as_ref() == b"id" && a.key.as_ref() != b"id" {
                        let rid = String::from_utf8_lossy(&a.value).into_owned();
                        if let Some(t) = targets
                            .iter()
                            .find(|(id, _)| *id == rid)
                            .map(|(_, t)| t.clone())
                        {
                            order.push(format!(
                                "ppt/{}",
                                t.trim_start_matches("/ppt/").trim_start_matches("./")
                            ));
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut out = String::new();
    for (i, part) in order.iter().enumerate() {
        let xml = match read_part(zip, part, budget) {
            Ok(x) => x,
            Err(_) => continue,
        };
        out.push_str(&format!("## Slide {}\n", i + 1));
        out.push_str(xml_text(&xml, OdfOrOoxml::Ooxml, deadline)?.trim());
        out.push_str("\n\n");
    }
    Ok(out)
}

/// `(Id, Target)` pairs of a `.rels` part.
fn relationship_targets(rels: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(rels);
    while let Ok(ev) = reader.read_event() {
        match ev {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                let mut id = None;
                let mut target = None;
                for a in e.attributes().flatten() {
                    match a.key.as_ref() {
                        b"Id" => id = Some(String::from_utf8_lossy(&a.value).into_owned()),
                        b"Target" => target = Some(String::from_utf8_lossy(&a.value).into_owned()),
                        _ => {}
                    }
                }
                if let (Some(i), Some(t)) = (id, target) {
                    out.push((i, t));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Every sheet as tab-separated rows under a `## Sheet` heading. A ZIP
/// workbook is checked with the same limits first; calamine then reads
/// through the zip crate, which stops at each entry's declared size.
/// Loading one sheet can't be interrupted, which is why this runs in the
/// child process ([`document_text_isolated`]) that is killed at the deadline.
fn spreadsheet_text(path: &Path, deadline: Instant) -> Result<String, ExtractError> {
    use calamine::{open_workbook_auto, Reader as _};
    open_checked_zip(path)?;
    let mut wb =
        open_workbook_auto(path).map_err(|e| err(format!("unreadable spreadsheet: {e}")))?;
    let mut out = String::new();
    for sheet in wb.sheet_names() {
        if Instant::now() > deadline || out.len() > MAX_TEXT_BYTES {
            break;
        }
        let Ok(range) = wb.worksheet_range(&sheet) else {
            continue;
        };
        out.push_str(&format!("## {sheet}\n"));
        for row in range.rows() {
            let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
            out.push_str(cells.join("\t").trim_end());
            out.push('\n');
            if out.len() > MAX_TEXT_BYTES || Instant::now() > deadline {
                break;
            }
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    /// Run as a child by `slow_child_is_killed_at_the_deadline`.
    #[test]
    #[ignore]
    fn sleeping_child() {
        std::thread::sleep(Duration::from_secs(20));
    }

    #[test]
    fn slow_child_is_killed_at_the_deadline() {
        let harness = std::env::current_exe().unwrap();
        let mut cmd = std::process::Command::new(harness);
        cmd.args([
            "--ignored",
            "--exact",
            "backend::attachments::extract::tests::sleeping_child",
        ]);
        let started = Instant::now();
        assert_eq!(
            run_with_deadline(cmd, Duration::from_millis(500)),
            ChildRun::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn child_output_reports_text_or_why_not() {
        let d = tempfile::tempdir().unwrap();
        let rtf = d.path().join("a.rtf");
        std::fs::write(&rtf, r"{\rtf1 Hello\par}").unwrap();
        let ok = document_text_isolated(&rtf, FileKind::Word, "a.rtf")
            .unwrap()
            .unwrap();
        assert!(ok.contains("Hello"), "{ok}");
        let broken = d.path().join("b.docx");
        std::fs::write(&broken, b"PK\x03\x04 not really").unwrap();
        assert!(
            document_text_isolated(&broken, FileKind::Word, "b.docx").is_ok_and(|t| t.is_none())
        );
        assert_eq!(child_output("text", "archive", &rtf, "a.rtf"), None);
    }

    fn zip_file(dir: &Path, name: &str, parts: &[(&str, &str)]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, body) in parts {
            z.start_file(*n, SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        p
    }

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    #[test]
    fn docx_paragraphs_runs_tabs_and_entities() {
        let d = tempfile::tempdir().unwrap();
        let doc = format!(
            r#"<w:document {W}><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p>
<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b &amp; c</w:t></w:r></w:p>
</w:body></w:document>"#
        );
        let hdr = format!(r#"<w:hdr {W}><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"#);
        let p = zip_file(
            d.path(),
            "r.docx",
            &[("word/document.xml", &doc), ("word/header1.xml", &hdr)],
        );
        let text = office_text(&p, FileKind::Word, "r.docx").unwrap().unwrap();
        assert!(text.starts_with("Text extracted from r.docx by AgentMux."));
        assert!(text.contains("Hello world\na\tb & c"), "{text}");
        assert!(text.contains("Header text"));
    }

    #[test]
    fn pptx_follows_presentation_order() {
        let d = tempfile::tempdir().unwrap();
        let pres = r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="257" r:id="rId3"/><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#;
        let rels = r#"<Relationships><Relationship Id="rId2" Target="slides/slide1.xml"/><Relationship Id="rId3" Target="slides/slide2.xml"/></Relationships>"#;
        let s1 = r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>First file, second slide</a:t></a:r></a:p></p:sld>"#;
        let s2 = r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Second file, first slide</a:t></a:r></a:p></p:sld>"#;
        let p = zip_file(
            d.path(),
            "deck.pptx",
            &[
                ("ppt/presentation.xml", pres),
                ("ppt/_rels/presentation.xml.rels", rels),
                ("ppt/slides/slide1.xml", s1),
                ("ppt/slides/slide2.xml", s2),
            ],
        );
        let text = office_text(&p, FileKind::PowerPoint, "deck.pptx")
            .unwrap()
            .unwrap();
        let first = text.find("Second file, first slide").unwrap();
        let second = text.find("First file, second slide").unwrap();
        assert!(first < second, "{text}");
        assert!(text.contains("## Slide 1\nSecond file"));
    }

    #[test]
    fn odt_paragraphs() {
        let d = tempfile::tempdir().unwrap();
        let content = r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><office:text><text:h>Title</text:h><text:p>Body<text:s/>text</text:p></office:text></office:body></office:document-content>"#;
        let p = zip_file(
            d.path(),
            "n.odt",
            &[
                ("mimetype", "application/vnd.oasis.opendocument.text"),
                ("content.xml", content),
            ],
        );
        let text = office_text(&p, FileKind::Word, "n.odt").unwrap().unwrap();
        assert!(text.contains("Title\nBody text"), "{text}");
    }

    #[test]
    fn custom_entities_are_never_expanded() {
        let xml = r#"<!DOCTYPE x [<!ENTITY a "AAAAAAAAAA"><!ENTITY b "&a;&a;&a;">]><w:document xmlns:w="w"><w:p><w:r><w:t>x&b;y</w:t></w:r></w:p></w:document>"#;
        let text = xml_text(xml, OdfOrOoxml::Ooxml, Instant::now() + DEADLINE).unwrap();
        assert_eq!(text.trim(), "xy");
    }

    #[test]
    fn a_zip_bomb_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bomb.docx");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("word/document.xml", SimpleFileOptions::default())
            .unwrap();
        // 20 MB of zeros compresses about 1000:1.
        z.write_all(&vec![b'0'; 20 * 1024 * 1024]).unwrap();
        z.finish().unwrap();
        let e = office_text(&p, FileKind::Word, "bomb.docx").unwrap_err();
        assert!(e.0.contains("zip bomb"), "{e}");
    }

    #[test]
    fn macros_are_detected_from_the_package() {
        let d = tempfile::tempdir().unwrap();
        let plain = zip_file(
            d.path(),
            "a.docx",
            &[
                ("[Content_Types].xml", "<Types/>"),
                ("word/document.xml", "<x/>"),
            ],
        );
        let with = zip_file(
            d.path(),
            "b.docx",
            &[
                ("[Content_Types].xml", "<Types/>"),
                ("word/document.xml", "<x/>"),
                ("word/vbaProject.bin", "x"),
            ],
        );
        assert!(!has_macros(&plain));
        assert!(has_macros(&with), "a .docx can still carry a VBA project");
    }

    #[test]
    fn preview_is_the_first_lines_cut_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("log.txt");
        let body: String = (0..100).map(|i| format!("line {i} é\n")).collect();
        std::fs::write(&p, body).unwrap();
        let prev = text_preview(&p).unwrap();
        assert_eq!(prev.lines().count(), PREVIEW_LINES);
        assert!(prev.starts_with("line 0 é"));
    }

    #[test]
    fn rtf_text_skips_tables_and_decodes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("n.rtf");
        std::fs::write(
            &p,
            r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}{\*\generator Word;}\f0 Hello\tab world\par Caf\'e9 \u8364? done\par}",
        )
        .unwrap();
        let t = rtf_text(&p, "n.rtf").unwrap();
        assert!(t.contains("Hello\tworld\nCafé € done"), "{t}");
        assert!(!t.contains("Arial") && !t.contains("Word;"), "{t}");
    }

    #[test]
    fn pdf_page_count_and_garbage() {
        let d = tempfile::tempdir().unwrap();
        let bad = d.path().join("bad.pdf");
        std::fs::write(&bad, b"%PDF-1.4\nnot really a pdf").unwrap();
        assert_eq!(pdf_facts_in_process(&bad, "bad.pdf"), PdfFacts::default());

        let good = d.path().join("two.pdf");
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for _ in 0..2 {
            let page =
                doc.add_object(lopdf::dictionary! { "Type" => "Page", "Parent" => pages_id });
            kids.push(page.into());
        }
        doc.objects.insert(
            pages_id,
            lopdf::Object::Dictionary(
                lopdf::dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 },
            ),
        );
        let catalog =
            doc.add_object(lopdf::dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(&good).unwrap();
        let facts = pdf_facts_in_process(&good, "two.pdf");
        assert_eq!(facts.pages, Some(2));
        // Pages with no content have no text.
        assert_eq!(facts.text, None);
    }
}
