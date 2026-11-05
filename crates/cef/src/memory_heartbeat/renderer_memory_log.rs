// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A timeline of every renderer's memory in the host log (task #23799, follow-up 1).
//
// The recycle governor measures app windows only, and only logs when it acts, so a renderer
// that grows for a day leaves no trace of how it got big or which one it was. Every few minutes
// the memory heartbeat calls [`log`], which writes one line per renderer process (app windows,
// browser panes, pool windows, and the processes site isolation gives a pane's cross-site
// iframes): its pid, role, the origin it shows and its private bytes. Logging only: nothing here
// changes when or whether a renderer is recycled.
//
// Not on Linux: the renderer map holds the sandbox-namespace PID there, which only srv
// translates, so a match against host PIDs could name another process (see
// `commands::recycle::renderer_mb`).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::collections::{BTreeMap, HashMap};

use agentmux_procstats::ProcInfo;

use crate::renderer_map::RendererEntry;
use crate::state::AppState;

/// How often the heartbeat (every 20 s) writes the timeline: every 15th tick, i.e. five minutes.
const EVERY_TICKS: u64 = 15;

/// The heartbeat's `n`th tick (from 0): write the timeline when it is due.
pub(super) fn tick(state: &AppState, n: u64) {
    if n % EVERY_TICKS == 0 {
        log(state);
    }
}

/// The longest origin written to the log.
const MAX_ORIGIN: usize = 120;

/// One renderer process, as logged.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    pid: u32,
    /// `app-window`, `browser-pane` or `pool-window`; `-frame` is added when the process serves
    /// only frames of such a browser (a cross-site iframe), not its page.
    role: String,
    /// The browsers it serves (a window's or pane's label); more than one when they share it.
    labels: Vec<String>,
    /// The origin (scheme, host and port: never a path or query) of each browser it serves.
    origin: String,
    private_bytes: Option<u64>,
}

/// `https://example.com:8443` for a page URL, with no user info, path, query or fragment. A URL
/// without an authority (`data:`, `about:blank`) is its scheme alone.
pub(crate) fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once(':') else { return String::new() };
    let scheme = scheme.to_ascii_lowercase();
    let Some(rest) = rest.strip_prefix("//") else { return format!("{scheme}:") };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let origin = if host.is_empty() { format!("{scheme}:") } else { format!("{scheme}://{host}") };
    origin.chars().take(MAX_ORIGIN).collect()
}

/// What the processes in `entries` are, largest first. `origins` is each browser's page origin by
/// label; a process the OS could not measure is listed without a size.
fn describe(entries: &[RendererEntry], procs: &[ProcInfo], origins: &HashMap<String, String>) -> Vec<Line> {
    let private: HashMap<u32, Option<u64>> = procs.iter().map(|p| (p.pid, p.mem_private)).collect();
    let mut by_pid: BTreeMap<u32, Vec<&RendererEntry>> = BTreeMap::new();
    for e in entries {
        by_pid.entry(e.pid).or_default().push(e);
    }
    let mut lines: Vec<Line> = by_pid
        .into_iter()
        .map(|(pid, served)| {
            let base = if served.iter().any(|e| e.block_id.is_some()) {
                "browser-pane"
            } else if served.iter().all(|e| e.pool) {
                "pool-window"
            } else {
                "app-window"
            };
            let role = if served.iter().any(|e| e.main) { base.to_string() } else { format!("{base}-frame") };
            let mut labels: Vec<String> = served.iter().map(|e| e.label.clone()).collect();
            labels.sort();
            labels.dedup();
            let mut seen: Vec<&str> = labels.iter().filter_map(|l| origins.get(l)).map(String::as_str).filter(|o| !o.is_empty()).collect();
            seen.sort_unstable();
            seen.dedup();
            Line { pid, role, labels, origin: seen.join(","), private_bytes: private.get(&pid).copied().flatten() }
        })
        .collect();
    lines.sort_by(|a, b| b.private_bytes.cmp(&a.private_bytes).then(a.pid.cmp(&b.pid)));
    lines
}

fn mb(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

fn text(l: &Line) -> String {
    let size = l.private_bytes.map_or("size unknown".to_string(), |b| format!("{} MB private", mb(b)));
    let origin = if l.origin.is_empty() { "no origin".to_string() } else { l.origin.clone() };
    format!("[renderer-mem] pid {} {} {} ({}): {size}", l.pid, l.role, origin, l.labels.join(","))
}

fn summary(lines: &[Line]) -> String {
    let total: u64 = lines.iter().filter_map(|l| l.private_bytes).sum();
    format!("[renderer-mem] {} renderers, {} MB private in total", lines.len(), mb(total))
}

/// Each browser's page origin, by label.
#[cfg(not(target_os = "linux"))]
fn origins_by_label(state: &AppState) -> HashMap<String, String> {
    use cef::*;
    state
        .list_browsers()
        .into_iter()
        .filter_map(|(label, browser)| {
            let url = CefString::from(&browser.main_frame()?.url()).to_string();
            Some((label, origin_of(&url)))
        })
        .collect()
}

/// Write one line per renderer process, and a total, to the host log.
#[cfg(not(target_os = "linux"))]
fn log(state: &AppState) {
    let entries = crate::renderer_map::snapshot(state);
    if entries.is_empty() {
        return;
    }
    let Ok(procs) = agentmux_procstats::snapshot() else { return };
    let lines = describe(&entries, &procs, &origins_by_label(state));
    for l in &lines {
        tracing::info!(
            target: "renderer_mem",
            pid = l.pid,
            role = %l.role,
            labels = %l.labels.join(","),
            origin = %l.origin,
            private_bytes = l.private_bytes,
            "{}",
            text(l)
        );
    }
    tracing::info!(target: "renderer_mem", renderers = lines.len(), "{}", summary(&lines));
}

#[cfg(target_os = "linux")]
fn log(_state: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, label: &str, block_id: Option<&str>, main: bool, pool: bool) -> RendererEntry {
        RendererEntry { pid, label: label.into(), main, block_id: block_id.map(Into::into), pool, window_id: None }
    }

    fn proc(pid: u32, private: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: None,
            start_key: 0,
            started_at_ms: None,
            name: "agentmux-cef".into(),
            cpu_ns: None,
            mem_private: private,
            mem_resident: None,
            mem_commit: None,
        }
    }

    fn origins(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(l, o)| (l.to_string(), o.to_string())).collect()
    }

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn an_origin_keeps_no_path_query_or_credentials() {
        assert_eq!(origin_of("https://example.com/a/b?token=secret#x"), "https://example.com");
        assert_eq!(origin_of("HTTP://user:pw@Host.test:8080/p"), "http://Host.test:8080");
        assert_eq!(origin_of("http://127.0.0.1:41234/?windowLabel=main"), "http://127.0.0.1:41234");
        assert_eq!(origin_of("data:text/html,<p>secret</p>"), "data:");
        assert_eq!(origin_of("about:blank"), "about:");
        assert_eq!(origin_of("not a url"), "");
        assert_eq!(origin_of("file:///C:/notes.txt"), "file:");
        assert!(origin_of(&format!("https://{}.test/", "a".repeat(500))).chars().count() <= MAX_ORIGIN);
    }

    #[test]
    fn every_renderer_is_a_line_largest_first_with_its_role_and_origin() {
        let entries = [
            entry(10, "main", None, true, false),
            entry(20, "browser-pane-a-1", Some("a"), true, false),
            entry(21, "browser-pane-a-1", Some("a"), false, false),
            entry(30, "window-pool-3", None, true, true),
        ];
        let procs = [proc(10, Some(600 * MIB)), proc(20, Some(15_000 * MIB)), proc(21, Some(40 * MIB)), proc(30, Some(90 * MIB))];
        let o = origins(&[("main", "http://127.0.0.1:41234"), ("browser-pane-a-1", "https://dash.example.com"), ("window-pool-3", "about:")]);
        let lines = describe(&entries, &procs, &o);
        let seen: Vec<(u32, &str, &str)> = lines.iter().map(|l| (l.pid, l.role.as_str(), l.origin.as_str())).collect();
        assert_eq!(
            seen,
            vec![
                (20, "browser-pane", "https://dash.example.com"),
                (10, "app-window", "http://127.0.0.1:41234"),
                (30, "pool-window", "about:"),
                (21, "browser-pane-frame", "https://dash.example.com"),
            ]
        );
        assert_eq!(lines[0].private_bytes, Some(15_000 * MIB));
        assert!(text(&lines[0]).contains("pid 20 browser-pane https://dash.example.com (browser-pane-a-1): 15000 MB private"), "{}", text(&lines[0]));
    }

    #[test]
    fn a_renderer_two_browsers_share_is_one_line_naming_both() {
        let entries = [entry(10, "main", None, true, false), entry(10, "browser-pane-a-1", Some("a"), true, false)];
        let lines = describe(&entries, &[proc(10, Some(300 * MIB))], &origins(&[("main", "http://127.0.0.1:1"), ("browser-pane-a-1", "http://127.0.0.1:1")]));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].labels, vec!["browser-pane-a-1".to_string(), "main".to_string()]);
        assert_eq!(lines[0].origin, "http://127.0.0.1:1", "the same origin once");
    }

    #[test]
    fn an_unmeasured_renderer_is_listed_without_a_size_and_counts_for_nothing() {
        let entries = [entry(10, "main", None, true, false), entry(11, "browser-pane-b-1", Some("b"), true, false)];
        let lines = describe(&entries, &[proc(10, Some(500 * MIB)), proc(11, None)], &HashMap::new());
        assert_eq!(lines.len(), 2);
        let unmeasured = lines.iter().find(|l| l.pid == 11).unwrap();
        assert_eq!(unmeasured.private_bytes, None);
        assert!(text(unmeasured).ends_with("size unknown") && text(unmeasured).contains("no origin"), "{}", text(unmeasured));
        assert_eq!(summary(&lines), "[renderer-mem] 2 renderers, 500 MB private in total");
        // A renderer the process list doesn't show at all is unmeasured too.
        assert_eq!(describe(&entries[..1], &[], &HashMap::new())[0].private_bytes, None);
    }
}
