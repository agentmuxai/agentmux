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
/// With a level or a text asked for, more of the file is searched, since the lines wanted may be far back among
/// noise. A bound on the work, and the answer says how much was searched.
const FILTERED_READ_BYTES: u64 = 4 * 1024 * 1024;
/// The longest text a line can be asked to contain.
const MAX_CONTAINS: usize = 200;
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

/// Every whole line of `bytes`, masked. The whole text is masked before it is split, so a private key whose
/// BEGIN and END lines are many lines apart is hidden as one block. When `from_middle` the first line is a fragment
/// of a longer one and is dropped.
fn window_lines(bytes: &[u8], from_middle: bool) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let text = if from_middle { without_open_key(&text) } else { &text };
    let masked = agentmux_common::pii_mask::mask_short_text(text);
    let mut lines: Vec<&str> = masked.lines().collect();
    if from_middle && !lines.is_empty() {
        lines.remove(0);
    }
    lines.into_iter().map(str::to_string).collect()
}

/// A log line's level, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// `level` as a caller names it.
fn parse_level(name: &str) -> Option<Level> {
    match name.trim().to_ascii_lowercase().as_str() {
        "debug" => Some(Level::Debug),
        "info" => Some(Level::Info),
        "warn" => Some(Level::Warn),
        "error" => Some(Level::Error),
        _ => None,
    }
}

/// The level a log line carries: the logs are JSON lines with `"level":"INFO"` near the start. A line
/// that has none (a wrapped continuation, text from a child process) has no level.
fn level_of(line: &str) -> Option<Level> {
    const KEY: &[u8] = b"\"level\":\"";
    // Bytes, not a str slice: byte 160 may fall inside a multi-byte character of a long message.
    let bytes = line.as_bytes();
    let head = &bytes[..bytes.len().min(160)];
    let from = head.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
    let name: Vec<u8> = head[from..].iter().copied().take_while(|b| *b != b'"').collect();
    match String::from_utf8_lossy(&name).to_ascii_uppercase().as_str() {
        "TRACE" => Some(Level::Trace),
        "DEBUG" => Some(Level::Debug),
        "INFO" => Some(Level::Info),
        "WARN" => Some(Level::Warn),
        "ERROR" => Some(Level::Error),
        _ => None,
    }
}

/// What a caller wants out of the lines read: a minimum level and a text a line must contain. Both must hold.
#[derive(Debug, Default)]
struct Filter {
    min_level: Option<Level>,
    /// Lowercased: the match ignores case.
    contains: Option<String>,
}

impl Filter {
    fn is_empty(&self) -> bool {
        self.min_level.is_none() && self.contains.is_none()
    }

    /// Whether `line` is wanted. With a level asked for, a line with no level is not.
    fn keeps(&self, line: &str) -> bool {
        if let Some(min) = self.min_level {
            if !level_of(line).is_some_and(|l| l >= min) {
                return false;
            }
        }
        match &self.contains {
            Some(text) => line.to_lowercase().contains(text.as_str()),
            None => true,
        }
    }

    fn from_params(params: &Value) -> Result<Self, RemoteRefusal> {
        let min_level = match params.get("level").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            None => None,
            Some(name) => Some(
                parse_level(name).ok_or_else(|| RemoteRefusal::failed(format!("level is \"debug\", \"info\", \"warn\" or \"error\", not {name:?}")))?,
            ),
        };
        let contains = match params.get("contains").and_then(Value::as_str) {
            None => None,
            Some(text) if text.is_empty() => None,
            Some(text) if text.chars().count() > MAX_CONTAINS => {
                return Err(RemoteRefusal::failed(format!("contains is at most {MAX_CONTAINS} characters")))
            }
            Some(text) => Some(text.to_lowercase()),
        };
        Ok(Self { min_level, contains })
    }
}

/// The last `n` of the lines that pass `filter`, and how many passed. The filter comes first, then the cut: so
/// "the last 100 warnings that mention store" is the last 100 of the warnings, not the warnings among the last 100.
fn pick(lines: Vec<String>, filter: &Filter, n: usize) -> (Vec<String>, usize) {
    let kept: Vec<String> = if filter.is_empty() { lines } else { lines.into_iter().filter(|l| filter.keeps(l)).collect() };
    let matched = kept.len();
    let start = matched.saturating_sub(n);
    (kept[start..].to_vec(), matched)
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

/// The last `window` bytes of the file, and whether that is only part of it.
fn read_tail(path: &Path, window: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let from_middle = len > window;
    file.seek(SeekFrom::Start(len.saturating_sub(window)))?;
    let mut buf = Vec::new();
    file.take(window).read_to_end(&mut buf)?;
    Ok((buf, from_middle))
}

pub(super) async fn logs_tail(_state: &AppState, _caller: &RemoteCaller, params: Value) -> RemoteResult {
    tail(&params)
}

/// The tail of this machine's srv or host log, for `app.logs.tail` and for `GET /api/v1/app/logs`.
/// Lines are read, masked, filtered, cut to the last `lines`, and fitted to the answer's size, in that order.
/// The answer says how many lines passed the filter (`matched`), out of how many were searched (`searched`).
pub(crate) fn tail(params: &Value) -> RemoteResult {
    let which = params.get("which").and_then(Value::as_str).unwrap_or("srv");
    let prefix = prefix_of(which).ok_or_else(|| RemoteRefusal::failed(format!("which is \"srv\" or \"host\", not {which:?}")))?;
    let wanted = params.get("lines").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let filter = Filter::from_params(params)?;
    let dir = agentmux_common::data_paths::DataPaths::from_env()
        .map(|p| p.logs_dir)
        .ok_or_else(|| RemoteRefusal::failed("this AgentMux has no log folder"))?;
    let Some((path, modified)) = newest_log(&dir, prefix) else {
        return Err(RemoteRefusal::failed(format!("no {which} log file yet")));
    };
    let window = if filter.is_empty() { READ_BYTES } else { FILTERED_READ_BYTES };
    let (bytes, from_middle) = read_tail(&path, window).map_err(|e| RemoteRefusal::failed(format!("reading the {which} log: {e}")))?;
    let searched = window_lines(&bytes, from_middle);
    let searched_count = searched.len();
    let (picked, matched) = pick(searched, &filter, wanted);
    let (lines, cut) = fit(picked, MAX_TEXT_BYTES);
    let modified_ms = modified.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    Ok(json!({
        "which": which,
        "lines": lines.len(),
        "matched": matched,
        "searched": searched_count,
        "searched_bytes": bytes.len(),
        // Only the end of the file is read; true when its start was reached.
        "whole_file": !from_middle,
        "modified_ms": modified_ms,
        "shortened": cut,
        "text": lines.join("\n"),
    }))
}

/// `GET /api/v1/app/logs?log=srv|host&lines=N&level=warn&contains=store`: the same read for this machine's own agents
/// (container agents too: it goes through srv, no host file access is given out).
pub(crate) async fn handle_local_logs(
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut params = json!({ "which": query.get("log").cloned().unwrap_or_else(|| "srv".to_string()) });
    if let Some(n) = query.get("lines").and_then(|v| v.parse::<u64>().ok()) {
        params["lines"] = json!(n);
    }
    for key in ["level", "contains"] {
        if let Some(v) = query.get(key) {
            params[key] = json!(v);
        }
    }
    match tokio::task::spawn_blocking(move || tail(&params)).await {
        Ok(Ok(answer)) => axum::Json(answer).into_response(),
        Ok(Err(refusal)) => (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({ "error": refusal.message }))).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": format!("reading the log: {e}") }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last `n` whole lines of `bytes`, masked, with no filter.
    fn last_lines(bytes: &[u8], n: usize, from_middle: bool) -> Vec<String> {
        pick(window_lines(bytes, from_middle), &Filter::default(), n).0
    }

    fn line(level: &str, message: &str) -> String {
        format!(r#"{{"timestamp":"2026-11-12T11:00:00Z","level":"{level}","fields":{{"message":"{message}"}},"target":"t"}}"#)
    }

    fn picked(lines: &[String], level: Option<&str>, contains: Option<&str>, n: usize) -> (Vec<String>, usize) {
        let mut params = json!({});
        if let Some(l) = level {
            params["level"] = json!(l);
        }
        if let Some(c) = contains {
            params["contains"] = json!(c);
        }
        pick(lines.to_vec(), &Filter::from_params(&params).unwrap(), n)
    }

    #[test]
    fn the_filter_comes_before_the_cut() {
        // Twelve store warnings, each followed by twelve lines of noise: the last 10 lines hold none of them.
        let mut lines = Vec::new();
        for i in 0..12 {
            lines.push(line("WARN", &format!("store update {i}")));
            for _ in 0..12 {
                lines.push(line("INFO", "[fe] [service] window.ListClosedWindows | 3ms"));
            }
        }
        let (last_ten, matched) = picked(&lines, None, None, 10);
        assert_eq!((last_ten.len(), matched), (10, 156));
        assert!(!last_ten.iter().any(|l| l.contains("store update")), "the plain tail is all noise");

        let (warnings, matched) = picked(&lines, Some("warn"), Some("STORE"), 3);
        assert_eq!(matched, 12, "all twelve passed the filter");
        assert_eq!(warnings.len(), 3, "and the cut kept the last three of them");
        assert!(warnings[0].contains("store update 9") && warnings[2].contains("store update 11"), "{warnings:?}");
    }

    #[test]
    fn a_minimum_level_keeps_that_level_and_above_and_drops_lines_with_none() {
        let lines = vec![
            line("DEBUG", "d"),
            line("INFO", "i"),
            line("WARN", "w"),
            line("ERROR", "e"),
            "a wrapped continuation with no level".to_string(),
        ];
        let levels = |min: &str| picked(&lines, Some(min), None, 10).0.iter().map(|l| level_of(l)).collect::<Vec<_>>();
        assert_eq!(levels("error"), [Some(Level::Error)]);
        assert_eq!(levels("warn"), [Some(Level::Warn), Some(Level::Error)]);
        assert_eq!(levels("info"), [Some(Level::Info), Some(Level::Warn), Some(Level::Error)]);
        assert_eq!(picked(&lines, Some("debug"), None, 10).1, 4, "a line with no level is not a debug line");
        assert_eq!(picked(&lines, None, None, 10).1, 5, "with no level asked for, it stays");
    }

    #[test]
    fn a_long_line_with_non_ascii_text_still_has_its_level() {
        // The level sits early; byte 160 falls inside a multi-byte character of the message after it.
        let message = format!("{}é{}", "x".repeat(87), "y".repeat(40));
        let text = line("WARN", &message);
        assert!(!text.is_char_boundary(160), "the test line must have a character straddling byte 160");
        assert_eq!(level_of(&text), Some(Level::Warn));
        assert_eq!(picked(&[text], Some("warn"), None, 10).1, 1);
    }

    #[test]
    fn the_text_match_ignores_case_and_both_filters_must_hold() {
        let lines = vec![line("WARN", "Store update failed"), line("INFO", "store update ok"), line("WARN", "network down")];
        assert_eq!(picked(&lines, None, Some("STORE UPDATE"), 10).1, 2);
        assert_eq!(picked(&lines, Some("warn"), Some("store"), 10).1, 1, "level and text together");
        assert_eq!(picked(&lines, Some("error"), Some("store"), 10).1, 0);
    }

    #[test]
    fn a_secret_is_masked_before_the_match_so_it_cannot_be_probed() {
        let token = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        let lines = window_lines(format!("{}\n", line("INFO", &format!("auth {token} sent"))).as_bytes(), false);
        assert_eq!(picked(&lines, None, Some("ghp_a1B2"), 10).1, 0, "the token is gone from the line the match sees");
        assert_eq!(picked(&lines, None, Some("auth"), 10).1, 1);
    }

    #[test]
    fn a_bad_level_or_too_long_a_text_is_refused_and_empty_ones_are_no_filter() {
        assert!(Filter::from_params(&json!({ "level": "loud" })).is_err());
        assert!(Filter::from_params(&json!({ "contains": "x".repeat(MAX_CONTAINS + 1) })).is_err());
        assert!(Filter::from_params(&json!({ "level": "", "contains": "" })).unwrap().is_empty());
        assert!(Filter::from_params(&json!({})).unwrap().is_empty());
        assert_eq!(parse_level(" Warn "), Some(Level::Warn));
    }

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
