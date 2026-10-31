// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.logs.tail`: the last lines of this AgentMux's own srv or host log (spec §4.3), a read.
//! Token shapes, private keys and card or ID numbers are hidden line by line, and the answer
//! carries no path: only the lines and when the file was last written.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};

const DEFAULT_LINES: usize = 200;
const MAX_LINES: usize = 1000;
/// How much of the end of a file is read: enough for `MAX_LINES` long lines, and a bound on the work.
const READ_BYTES: u64 = 1024 * 1024;
/// A cap on the text of one answer; the oldest lines go first.
const MAX_TEXT_BYTES: usize = 256 * 1024;

/// The log file name prefix of `which`.
fn prefix_of(which: &str) -> Option<&'static str> {
    match which {
        "srv" => Some("agentmuxsrv-"),
        "host" => Some("agentmux-host-"),
        _ => None,
    }
}

/// The most recently written log file in `dir` whose name starts with `prefix`.
fn newest_log(dir: &Path, prefix: &str) -> Option<(PathBuf, std::time::SystemTime)> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .max_by_key(|(_, modified)| *modified)
}

// Written in two parts so this file holds no key marker of its own.
const KEY_BEGIN: &str = concat!("-----", "BEGIN ");
const KEY_END: &str = concat!("-----", "END ");

/// `text` read from the middle of a file may start inside a private key: its BEGIN line is before the window, so
/// the key's body lines would not match the key mask. When an END marker comes before any BEGIN marker, everything
/// up to the end of that line is dropped.
fn without_open_key(text: &str) -> &str {
    let Some(end) = text.find(KEY_END) else { return text };
    if text.find(KEY_BEGIN).is_some_and(|begin| begin < end) {
        return text;
    }
    text[end..].find('\n').map_or("", |nl| &text[end + nl + 1..])
}

/// The last `n` whole lines of `bytes`, masked. The whole text is masked before it is split, so a private key whose
/// BEGIN and END lines are many lines apart is hidden as one block. When `from_middle` the first line is a fragment
/// of a longer one and is dropped.
fn last_lines(bytes: &[u8], n: usize, from_middle: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let text = if from_middle { without_open_key(&text) } else { &text };
    let masked = agentmux_common::pii_mask::mask_short_text(text);
    let mut lines: Vec<&str> = masked.lines().collect();
    if from_middle && !lines.is_empty() {
        lines.remove(0);
    }
    let start = lines.len().saturating_sub(n);
    lines[start..].iter().map(|l| l.to_string()).collect()
}

/// Keeps the newest lines whose text fits `max` bytes; says whether any were left out.
fn fit(lines: Vec<String>, max: usize) -> (Vec<String>, bool) {
    let mut total = 0usize;
    let mut keep = 0usize;
    for line in lines.iter().rev() {
        total += line.len() + 1;
        if total > max {
            break;
        }
        keep += 1;
    }
    let skipped = lines.len() - keep;
    (lines[skipped..].to_vec(), skipped > 0)
}

fn read_tail(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let from_middle = len > READ_BYTES;
    file.seek(SeekFrom::Start(len.saturating_sub(READ_BYTES)))?;
    let mut buf = Vec::new();
    file.take(READ_BYTES).read_to_end(&mut buf)?;
    Ok((buf, from_middle))
}

pub(super) async fn logs_tail(_state: &AppState, _caller: &RemoteCaller, params: Value) -> RemoteResult {
    let which = params.get("which").and_then(Value::as_str).unwrap_or("srv");
    let prefix = prefix_of(which).ok_or_else(|| RemoteRefusal::failed(format!("which is \"srv\" or \"host\", not {which:?}")))?;
    let wanted = params.get("lines").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let dir = agentmux_common::data_paths::DataPaths::from_env()
        .map(|p| p.logs_dir)
        .ok_or_else(|| RemoteRefusal::failed("this AgentMux has no log folder"))?;
    let Some((path, modified)) = newest_log(&dir, prefix) else {
        return Err(RemoteRefusal::failed(format!("no {which} log file yet")));
    };
    let (bytes, from_middle) = read_tail(&path).map_err(|e| RemoteRefusal::failed(format!("reading the {which} log: {e}")))?;
    let (lines, cut) = fit(last_lines(&bytes, wanted, from_middle), MAX_TEXT_BYTES);
    let modified_ms = modified.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    Ok(json!({
        "which": which,
        "lines": lines.len(),
        "modified_ms": modified_ms,
        "shortened": cut,
        "text": lines.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_lines_come_back_and_a_cut_first_line_is_dropped() {
        let bytes = b"cut off\none\ntwo\nthree\n";
        assert_eq!(last_lines(bytes, 2, true), ["two", "three"]);
        assert_eq!(last_lines(bytes, 10, true), ["one", "two", "three"]);
        assert_eq!(last_lines(bytes, 10, false), ["cut off", "one", "two", "three"]);
    }

    #[test]
    fn a_token_in_a_line_is_hidden() {
        let line = format!("auth ghp_{} sent", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        let out = last_lines(line.as_bytes(), 1, false);
        assert!(!out[0].contains("ghp_a1B2"), "{}", out[0]);
        assert!(out[0].starts_with("auth ") && out[0].ends_with(" sent"), "{}", out[0]);
    }

    #[test]
    fn a_private_key_spread_over_lines_is_hidden_whole() {
        // The markers are built here so this file holds no key block of its own.
        let (begin, end) = (format!("-----{} PRIVATE KEY-----", "BEGIN"), format!("-----{} PRIVATE KEY-----", "END"));
        let log = format!("before
{begin}
MIIEvQIBADANBgkqhkiG9w0BAQEFAASC
abcdEFGHijklMNOPqrstUVWXyz012345
{end}
after
");
        let out = last_lines(log.as_bytes(), 10, false).join("
");
        assert!(!out.contains("MIIEvQIBADAN") && !out.contains("abcdEFGH"), "{out}");
        assert!(out.starts_with("before") && out.ends_with("after"), "{out}");
    }

    #[test]
    fn a_key_that_began_before_the_window_is_cut_off_at_its_end() {
        let end = format!("{KEY_END}PRIVATE KEY-----");
        let log = format!("MIIEvQIBADANBgkqhkiG9w0BAQEFAASC
abcdEFGHijklMNOPqrstUVWXyz012345
{end}
kept one
kept two
");
        // Read from the middle of a file: the key's body is not shown.
        let out = last_lines(log.as_bytes(), 10, true).join("
");
        assert!(!out.contains("MIIEvQIBADAN") && !out.contains("abcdEFGH"), "{out}");
        assert_eq!(out, "kept two", "the first line after the cut is dropped as a fragment too: {out}");
        // A key whose BEGIN is in the window is left to the mask.
        let whole = format!("{KEY_BEGIN}PRIVATE KEY-----
body
{end}
after");
        assert_eq!(without_open_key(&whole), whole);
    }

    #[test]
    fn too_much_text_drops_the_oldest_lines() {
        let lines: Vec<String> = ["aaaa", "bbbb", "cccc"].iter().map(|s| s.to_string()).collect();
        assert_eq!(fit(lines.clone(), 100), (lines.clone(), false));
        assert_eq!(fit(lines, 10), (vec!["bbbb".to_string(), "cccc".to_string()], true));
    }

    #[test]
    fn the_newest_file_with_the_prefix_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agentmuxsrv-v1.log.2026-10-29"), "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.path().join("agentmuxsrv-v1.log.2026-10-30"), "new").unwrap();
        std::fs::write(dir.path().join("agentmux-host-v1.log.2026-10-30"), "host").unwrap();
        let (path, _) = newest_log(dir.path(), "agentmuxsrv-").unwrap();
        assert!(path.ends_with("agentmuxsrv-v1.log.2026-10-30"));
        assert!(newest_log(dir.path(), "nothing-").is_none());
    }

    #[test]
    fn an_unknown_log_is_refused_not_guessed() {
        assert!(prefix_of("srv").is_some() && prefix_of("host").is_some() && prefix_of("cef").is_none());
    }
}
