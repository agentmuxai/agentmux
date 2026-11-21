// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `muxspect browsers`: which page each renderer draws, how busy it is, and what the page is doing
//! (docs/specs/SPEC_RELEASE_BUILD_BROWSER_PAGE_DIAGNOSTIC_2026_11_13.md §3.4).
//!
//! The window host knows the pages (`page_probe`) and which renderer serves which browser
//! (`renderer_map`); the operating system knows each process's CPU. This joins the three, with no
//! I/O, so every case is a unit test: a renderer shared by several browsers, one that serves only a
//! frame, a pid present in only one sample, a page that did not answer, a Linux namespace pid.

use std::collections::{BTreeMap, HashMap, HashSet};

use agentmux_procstats::ProcInfo;
use serde::{Deserialize, Serialize};

use super::agentmux::RendererEntry;

/// The host's row for one browser (`crates/cef/src/browser_api/page_probe.rs`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PageProbe {
    pub label: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub has_focus: Option<bool>,
    #[serde(default)]
    pub animations: Option<u64>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub metrics: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    pub error: Option<String>,
}

const PANE: &str = "browser-pane";

/// The host's `page_probe` answer (`{ok, data: {browsers}}`) as rows; none for anything else, so an
/// older host just leaves the page facts out. A browser pane's row is cut down again here, whatever
/// the host sent: a person's page never contributes a title or an animation name.
pub fn parse_page_probe(answer: &serde_json::Value) -> Vec<PageProbe> {
    let mut rows: Vec<PageProbe> = answer
        .get("data")
        .and_then(|d| d.get("browsers"))
        .and_then(|r| serde_json::from_value(r.clone()).ok())
        .unwrap_or_default();
    for row in rows.iter_mut().filter(|r| r.role == PANE) {
        row.title = None;
        row.kinds.clear();
    }
    rows
}

/// One renderer's row (or, with no `pid`, a browser that has none: its page is asleep).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserRow {
    /// The process id as the OS lists it (on Linux the host's, not the sandbox namespace's).
    pub pid: Option<u32>,
    pub label: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How the page's facts were read: `answered`, `asleep`, `frozen`, `no_answer`, `failed`, or
    /// `unknown` when the host could not be asked.
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_focus: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animations: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// This renderer serves only a frame of the page (a cross-site iframe), not its main document.
    pub frame: bool,
    /// Other browsers the same renderer serves; its CPU is the process's and is not split.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared_with: Vec<String>,
    /// Percent of one core over the sample.
    pub cpu_pct: Option<f64>,
    /// Why there is no CPU figure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_note: Option<String>,
    /// With `detail`: percent of the sample the page's main thread spent in each kind of work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_thread_pct: Option<BTreeMap<String, f64>>,
}

/// The AgentMux processes that serve no page, for scale next to the page rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Others {
    pub processes: usize,
    pub cpu_pct: f64,
}

/// Two process snapshots `secs` apart.
pub struct Sample<'a> {
    pub before: &'a [ProcInfo],
    pub after: &'a [ProcInfo],
    pub secs: f64,
}

impl Sample<'_> {
    /// CPU percent of one core for `pid`, or why it has none.
    fn cpu(&self, pid: u32) -> Result<f64, &'static str> {
        let after = self.after.iter().find(|p| p.pid == pid).ok_or("not running at the end of the sample")?;
        let before = self.before.iter().find(|p| p.key() == after.key()).ok_or("started during the sample")?;
        match (before.cpu_ns, after.cpu_ns) {
            (Some(b), Some(a)) if self.secs > 0.0 => Ok(a.saturating_sub(b) as f64 / self.secs / 1e9 * 100.0),
            _ => Err("the OS gave no CPU time for it"),
        }
    }
}

/// Where the main thread's time went, as percent of the interval between the two reads. The page's
/// own clock (`Timestamp`) is the interval when both reads have it.
fn main_thread_pct(before: &BTreeMap<String, f64>, after: &BTreeMap<String, f64>, secs: f64) -> Option<BTreeMap<String, f64>> {
    let interval = match (before.get("Timestamp"), after.get("Timestamp")) {
        (Some(b), Some(a)) if a > b => a - b,
        _ => secs,
    };
    if interval <= 0.0 {
        return None;
    }
    let out: BTreeMap<String, f64> = after
        .iter()
        .filter(|(name, _)| name.as_str() != "Timestamp")
        .filter_map(|(name, a)| Some((name.clone(), ((a - before.get(name)?) / interval * 100.0).max(0.0))))
        .collect();
    (!out.is_empty()).then_some(out)
}

fn role_of(e: &RendererEntry) -> &'static str {
    if e.block_id.is_some() {
        PANE
    } else if e.pool {
        "pool-window"
    } else if e.window_id.is_some() {
        "app-window"
    } else {
        // Not positively AgentMux's own (the host reads such a page as a person's).
        PANE
    }
}

/// A row with the page's facts from `probe` (if the host gave one).
fn row(label: &str, pid: Option<u32>, entry: Option<&RendererEntry>, probe: Option<&PageProbe>) -> BrowserRow {
    let blank = PageProbe::default();
    let p = probe.unwrap_or(&blank);
    BrowserRow {
        pid,
        label: label.to_string(),
        role: if !p.role.is_empty() { p.role.clone() } else { entry.map_or("", |e| role_of(e)).to_string() },
        block_id: p.block_id.clone().or_else(|| entry.and_then(|e| e.block_id.clone())),
        window_id: p.window_id.clone().or_else(|| entry.and_then(|e| e.window_id.clone())),
        origin: p.origin.clone(),
        title: p.title.clone(),
        answer: if probe.is_some() { p.answer.clone() } else { "unknown".to_string() },
        visibility: p.visibility.clone(),
        has_focus: p.has_focus,
        animations: p.animations,
        kinds: p.kinds.clone(),
        frame: false,
        shared_with: Vec::new(),
        cpu_pct: None,
        cpu_note: None,
        main_thread_pct: None,
    }
}

/// One row per renderer, then one per browser that has none (its page is asleep), busiest first.
///
/// `host_pid` maps the pid a renderer reported for itself to the pid the OS lists (the same pid
/// except on Linux, where the sandbox's PID namespace makes the reported one small and local).
pub fn browser_rows(
    entries: &[RendererEntry],
    probes: &[PageProbe],
    before_probes: &[PageProbe],
    sample: &Sample<'_>,
    host_pid: &dyn Fn(u32) -> Option<u32>,
) -> Vec<BrowserRow> {
    let probe_of: HashMap<&str, &PageProbe> = probes.iter().map(|p| (p.label.as_str(), p)).collect();
    let before_of: HashMap<&str, &PageProbe> = before_probes.iter().map(|p| (p.label.as_str(), p)).collect();
    let mut by_pid: BTreeMap<u32, Vec<&RendererEntry>> = BTreeMap::new();
    for e in entries {
        by_pid.entry(e.pid).or_default().push(e);
    }
    let mut served: HashSet<&str> = HashSet::new();
    let mut rows: Vec<BrowserRow> = Vec::new();
    for (reported, es) in &by_pid {
        let mut labels: Vec<&str> = es.iter().map(|e| e.label.as_str()).collect();
        labels.sort_unstable();
        labels.dedup();
        // The browser whose own renderer this is; a renderer that serves only frames is shown under
        // the first browser it serves, marked as a frame.
        let main = es.iter().filter(|e| e.main).map(|e| e.label.as_str()).min();
        let primary = main.unwrap_or(labels[0]);
        let entry = es.iter().find(|e| e.label == primary).copied();
        served.extend(labels.iter().copied());
        let mut r = row(primary, host_pid(*reported), entry, probe_of.get(primary).copied());
        r.frame = main.is_none();
        r.shared_with = labels.iter().filter(|l| **l != primary).map(|l| l.to_string()).collect();
        match r.pid.map(|pid| sample.cpu(pid)) {
            Some(Ok(cpu)) => r.cpu_pct = Some(cpu),
            Some(Err(why)) => r.cpu_note = Some(why.to_string()),
            None => r.cpu_note = Some("no such process on this computer".to_string()),
        }
        if let (Some(b), Some(a)) = (before_of.get(primary).and_then(|p| p.metrics.as_ref()), probe_of.get(primary).and_then(|p| p.metrics.as_ref())) {
            r.main_thread_pct = main_thread_pct(b, a, sample.secs);
        }
        rows.push(r);
    }
    // A browser with no renderer: its page is asleep (or its renderer has not reported yet).
    let mut rest: Vec<&PageProbe> = probes.iter().filter(|p| !served.contains(p.label.as_str())).collect();
    rest.sort_by(|a, b| a.label.cmp(&b.label));
    rows.extend(rest.into_iter().map(|p| row(&p.label, None, None, Some(p))));
    rows.sort_by(|a, b| {
        let key = |r: &BrowserRow| r.cpu_pct.unwrap_or(-1.0);
        key(b).partial_cmp(&key(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.label.cmp(&b.label))
    });
    rows
}

/// The processes of this AgentMux instance: the launcher above `own_pid` and everything under it.
/// AgentMux's own executables carry the name, so another program a shell started is not climbed into.
pub fn instance_pids(snap: &[ProcInfo], own_pid: u32) -> HashSet<u32> {
    let by_pid: HashMap<u32, &ProcInfo> = snap.iter().map(|p| (p.pid, p)).collect();
    let parent = |p: &ProcInfo| p.ppid.and_then(|pp| by_pid.get(&pp).copied()).filter(|q| p.is_child_of(q));
    let ours = |p: &ProcInfo| p.name.to_ascii_lowercase().starts_with("agentmux");
    let Some(mut root) = by_pid.get(&own_pid).copied() else { return HashSet::new() };
    for _ in 0..16 {
        match parent(root).filter(|q| ours(q)) {
            Some(q) => root = q,
            None => break,
        }
    }
    snap.iter()
        .filter(|p| {
            let mut at = *p;
            for _ in 0..16 {
                if at.pid == root.pid {
                    return true;
                }
                match parent(at) {
                    Some(q) => at = q,
                    None => return false,
                }
            }
            false
        })
        .map(|p| p.pid)
        .collect()
}

/// The instance's processes that serve no page (GPU, network service, window host, srv, launcher),
/// and their CPU together, over processes measured in both samples.
pub fn others(instance: &HashSet<u32>, page_pids: &HashSet<u32>, sample: &Sample<'_>) -> Others {
    let rest: Vec<u32> = instance.iter().copied().filter(|p| !page_pids.contains(p)).collect();
    let cpu_pct: f64 = rest.iter().filter_map(|p| sample.cpu(*p).ok()).sum();
    Others { processes: rest.len(), cpu_pct }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SEC: u64 = 1_000_000_000;

    fn proc(pid: u32, ppid: u32, name: &str, cpu_secs: u64) -> ProcInfo {
        ProcInfo { pid, ppid: Some(ppid), start_key: pid as u64 * 10, started_at_ms: Some(1000 + pid as u64), name: name.into(), cpu_ns: Some(cpu_secs * SEC), mem_private: None, mem_resident: None, mem_commit: None }
    }

    fn entry(pid: u32, label: &str, main: bool, block_id: Option<&str>, pool: bool) -> RendererEntry {
        RendererEntry { pid, label: label.into(), main, block_id: block_id.map(Into::into), pool, window_id: None }
    }

    fn probe(label: &str, role: &str, answer: &str) -> PageProbe {
        PageProbe { label: label.into(), role: role.into(), answer: answer.into(), visibility: Some("visible".into()), animations: Some(0), ..PageProbe::default() }
    }

    fn rows(entries: &[RendererEntry], probes: &[PageProbe]) -> Vec<BrowserRow> {
        let before = [proc(10, 1, "agentmux-cef", 5), proc(20, 1, "agentmux-cef", 5), proc(30, 1, "agentmux-cef", 5)];
        let after = [proc(10, 1, "agentmux-cef", 8), proc(20, 1, "agentmux-cef", 5), proc(30, 1, "agentmux-cef", 6)];
        browser_rows(entries, probes, &[], &Sample { before: &before, after: &after, secs: 3.0 }, &Some)
    }

    #[test]
    fn cpu_is_the_process_time_over_the_sample() {
        let r = rows(&[entry(10, "main", true, None, false)], &[probe("main", "app-window", "answered")]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cpu_pct, Some(100.0));
        assert_eq!(r[0].role, "app-window");
        assert_eq!(r[0].visibility.as_deref(), Some("visible"));
    }

    #[test]
    fn a_shared_renderer_is_one_row_that_names_the_others_and_is_not_split() {
        let r = rows(
            &[entry(10, "pane-b", true, Some("b2"), false), entry(10, "pane-a", true, Some("b1"), false)],
            &[probe("pane-a", "browser-pane", "answered"), probe("pane-b", "browser-pane", "answered")],
        );
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].label.as_str(), r[0].shared_with.clone()), ("pane-a", vec!["pane-b".to_string()]));
        assert_eq!(r[0].cpu_pct, Some(100.0));
    }

    #[test]
    fn a_frame_only_renderer_is_a_frame_row_with_the_page_it_belongs_to() {
        let r = rows(&[entry(10, "pane-a", true, Some("b1"), false), entry(30, "pane-a", false, Some("b1"), false)], &[probe("pane-a", "browser-pane", "answered")]);
        assert_eq!(r.len(), 2);
        let frame = r.iter().find(|x| x.pid == Some(30)).unwrap();
        assert!(frame.frame && frame.label == "pane-a");
        assert!(!r.iter().find(|x| x.pid == Some(10)).unwrap().frame);
    }

    #[test]
    fn a_pid_in_only_one_sample_has_no_cpu_and_says_why() {
        let before = [proc(10, 1, "agentmux-cef", 5)];
        let after = [proc(10, 1, "agentmux-cef", 8), proc(20, 1, "agentmux-cef", 1)];
        let sample = Sample { before: &before, after: &after, secs: 3.0 };
        let r = browser_rows(&[entry(20, "new", true, None, false), entry(99, "gone", true, None, false)], &[], &[], &sample, &Some);
        let new = r.iter().find(|x| x.pid == Some(20)).unwrap();
        assert_eq!((new.cpu_pct, new.cpu_note.as_deref()), (None, Some("started during the sample")));
        assert_eq!(r.iter().find(|x| x.pid == Some(99)).unwrap().cpu_note.as_deref(), Some("not running at the end of the sample"));
    }

    #[test]
    fn a_page_that_did_not_answer_keeps_its_pid_and_cpu() {
        let r = rows(&[entry(10, "main", true, None, false)], &[probe("main", "app-window", "no_answer")]);
        assert_eq!((r[0].answer.as_str(), r[0].cpu_pct), ("no_answer", Some(100.0)));
    }

    #[test]
    fn a_browser_with_no_renderer_is_a_row_with_no_pid() {
        let r = rows(&[entry(10, "main", true, None, false)], &[probe("main", "app-window", "answered"), probe("pane-z", "browser-pane", "asleep")]);
        let asleep = r.iter().find(|x| x.label == "pane-z").unwrap();
        assert_eq!((asleep.pid, asleep.answer.as_str(), asleep.cpu_pct), (None, "asleep", None));
    }

    #[test]
    fn an_older_host_without_page_facts_still_gives_rows() {
        let r = rows(&[entry(10, "window-pool-1", true, None, true)], &[]);
        assert_eq!((r[0].role.as_str(), r[0].answer.as_str()), ("pool-window", "unknown"));
    }

    #[test]
    fn a_linux_namespace_pid_is_shown_as_the_hosts() {
        let before = [proc(4242, 1, "agentmux-cef", 0)];
        let after = [proc(4242, 1, "agentmux-cef", 3)];
        let sample = Sample { before: &before, after: &after, secs: 3.0 };
        let r = browser_rows(&[entry(7, "main", true, None, false)], &[], &[], &sample, &|p| (p == 7).then_some(4242));
        assert_eq!((r[0].pid, r[0].cpu_pct), (Some(4242), Some(100.0)));
    }

    #[test]
    fn rows_are_busiest_first() {
        let r = rows(&[entry(10, "a", true, None, false), entry(20, "b", true, None, false), entry(30, "c", true, None, false)], &[]);
        assert_eq!(r.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["a", "c", "b"]);
    }

    #[test]
    fn the_main_thread_split_uses_the_pages_own_clock() {
        let m = |t: f64, task: f64, script: f64| BTreeMap::from([("Timestamp".to_string(), t), ("TaskDuration".to_string(), task), ("ScriptDuration".to_string(), script)]);
        let got = main_thread_pct(&m(100.0, 1.0, 0.5), &m(102.0, 1.5, 1.5), 3.0).unwrap();
        assert_eq!((got["TaskDuration"], got["ScriptDuration"]), (25.0, 50.0));
        assert!(!got.contains_key("Timestamp"));
        assert!(main_thread_pct(&m(1.0, 0.0, 0.0), &BTreeMap::new(), 3.0).is_none());
    }

    #[test]
    fn a_pane_row_never_keeps_a_title_or_animation_names_whatever_the_host_sent() {
        let answer = json!({ "ok": true, "data": { "browsers": [
            { "label": "p", "role": "browser-pane", "answer": "answered", "title": "Bank", "kinds": ["CSSAnimation:x"], "origin": "https://a.test" },
            { "label": "main", "role": "app-window", "answer": "answered", "title": "AgentMux", "kinds": ["CSSAnimation:startup-spin"] },
        ] } });
        let rows = parse_page_probe(&answer);
        assert_eq!((rows[0].title.clone(), rows[0].kinds.len()), (None, 0));
        assert_eq!(rows[0].origin, "https://a.test");
        assert_eq!((rows[1].title.as_deref(), rows[1].kinds.len()), (Some("AgentMux"), 1));
        assert!(parse_page_probe(&json!({ "ok": false })).is_empty());
    }

    #[test]
    fn the_instance_is_the_launcher_and_what_runs_under_it_not_a_shell_above_it() {
        let snap = [
            proc(1, 0, "explorer", 0),
            proc(2, 1, "agentmux-launcher", 0),
            proc(3, 2, "agentmux-cef", 0),
            proc(4, 3, "agentmux-srv", 0),
            proc(5, 3, "agentmux-cef", 0),
            proc(6, 1, "other-tool", 0),
            proc(7, 99, "agentmux-cef", 0),
        ];
        let mut got: Vec<u32> = instance_pids(&snap, 4).into_iter().collect();
        got.sort_unstable();
        assert_eq!(got, vec![2, 3, 4, 5]);
        assert!(instance_pids(&snap, 500).is_empty());
    }

    #[test]
    fn the_rest_of_the_instance_is_summed_for_scale() {
        let before = [proc(2, 1, "agentmux-launcher", 0), proc(3, 2, "agentmux-cef", 0), proc(5, 3, "agentmux-cef", 0)];
        let after = [proc(2, 1, "agentmux-launcher", 0), proc(3, 2, "agentmux-cef", 3), proc(5, 3, "agentmux-cef", 6)];
        let instance: HashSet<u32> = [2, 3, 5].into();
        let o = others(&instance, &[5].into(), &Sample { before: &before, after: &after, secs: 3.0 });
        assert_eq!((o.processes, o.cpu_pct), (2, 100.0));
    }
}
