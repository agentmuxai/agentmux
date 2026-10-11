// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Overlap notes, the pure half
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.3, file half):
//! when an agent edits a file, which other agents have the same
//! (repository, repository-relative path) uncommitted or edited it in the
//! last [`EDITED_WINDOW_MS`]; the note that says so; and the rate limits that
//! keep those notes rare. The runtime half (when to check, fetching facts,
//! delivering) is `overlap_notes`.

use std::collections::HashMap;

use super::paths::{normalize_path, path_matches, relative_to};
use super::WorkFacts;

/// Another agent's edit counts for this long.
pub const EDITED_WINDOW_MS: u64 = 2 * 60 * 60 * 1000;

/// At most one note per (agent, other agent, repository, path) this often.
pub const PAIR_QUIET_MS: u64 = 2 * 60 * 60 * 1000;

/// At most [`WINDOW_NOTES`] notes per agent in this window; the next one is a
/// summary, then nothing until the window ends.
pub const WINDOW_MS: u64 = 10 * 60 * 1000;
pub const WINDOW_NOTES: usize = 3;

/// The same agent's edits of the same file are checked at most this often.
pub const RECHECK_MS: u64 = 60 * 1000;

/// The file an agent just edited, placed in a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedFile {
    /// Repository key ([`WorkFacts::repo_key`]).
    pub repo_key: String,
    /// How the note names the repository: `owner/repo`, else the clone's folder.
    pub repo: String,
    pub rel: String,
}

/// How another agent is working on the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlapKind {
    Dirty,
    Edited,
}

/// Another agent working on the same file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    pub agent: String,
    pub kind: OverlapKind,
    pub branch: Option<String>,
    /// That agent's last edit of the file (Unix ms), when known.
    pub edited_ms: Option<u64>,
}

/// Where `path` (absolute, as the edit tool named it) is: the editor's own
/// clone first, then any other known clone. `None` when it is in no known
/// repository, so there is nothing to compare across clones.
pub fn locate(path: &str, me: Option<&WorkFacts>, all: &[WorkFacts]) -> Option<EditedFile> {
    let abs = normalize_path(path);
    me.into_iter().chain(all.iter()).find_map(|f| {
        let rel = relative_to(&abs, f.repo_root.as_deref()?)?;
        if rel.is_empty() {
            return None;
        }
        let repo_key = f.repo_key()?;
        let repo = f.repo.clone().or_else(|| f.repo_root.clone())?;
        Some(EditedFile { repo_key, repo, rel })
    })
}

/// Every other agent with `file` uncommitted, or edited in the last
/// [`EDITED_WINDOW_MS`]. The editor itself (`me_agent`, by name in any
/// channel, so its own other clones and instances too) is left out. One
/// entry per agent, uncommitted first.
pub fn overlaps(file: &EditedFile, me_agent: &str, all: &[WorkFacts], now_ms: u64) -> Vec<Overlap> {
    let mut by_agent: HashMap<String, Overlap> = HashMap::new();
    for f in all {
        if f.agent.eq_ignore_ascii_case(me_agent) || f.repo_key().as_deref() != Some(file.repo_key.as_str()) {
            continue;
        }
        // The file itself, or an untracked folder (`dir/`) holding it.
        let dirty = f.dirty_files.iter().any(|d| path_matches(d, &file.rel));
        let edited_ms = f
            .recent_files
            .iter()
            .filter(|r| r.rel.as_deref().is_some_and(|rel| path_matches(rel, &file.rel)))
            .map(|r| r.ts_ms)
            .max();
        let recent = edited_ms.filter(|ts| now_ms.saturating_sub(*ts) <= EDITED_WINDOW_MS);
        let kind = match (dirty, recent) {
            (true, _) => OverlapKind::Dirty,
            (false, Some(_)) => OverlapKind::Edited,
            (false, None) => continue,
        };
        let candidate = Overlap { agent: f.agent.clone(), kind, branch: f.branch.clone(), edited_ms: recent };
        // The same agent in two channels or panes: the stronger, newer one.
        let key = f.agent.to_lowercase();
        match by_agent.get(&key) {
            Some(prev) if (prev.kind, std::cmp::Reverse(prev.edited_ms)) <= (kind, std::cmp::Reverse(recent)) => {}
            _ => {
                by_agent.insert(key, candidate);
            }
        }
    }
    let mut out: Vec<Overlap> = by_agent.into_values().collect();
    out.sort_by(|a, b| {
        a.kind.cmp(&b.kind).then_with(|| b.edited_ms.cmp(&a.edited_ms)).then_with(|| a.agent.cmp(&b.agent))
    });
    out
}

/// "just now", "1 minute ago", "25 minutes ago".
fn ago(now_ms: u64, ts_ms: u64) -> String {
    match now_ms.saturating_sub(ts_ms) / 60_000 {
        0 => "just now".to_string(),
        1 => "1 minute ago".to_string(),
        m => format!("{m} minutes ago"),
    }
}

/// What another agent did to the file: "has uncommitted changes in it",
/// "edited it 5 minutes ago".
fn doing(o: &Overlap, now_ms: u64) -> String {
    match (o.kind, o.edited_ms) {
        (OverlapKind::Dirty, _) => "has uncommitted changes in it".to_string(),
        (OverlapKind::Edited, Some(ts)) => format!("edited it {}", ago(now_ms, ts)),
        (OverlapKind::Edited, None) => "edited it recently".to_string(),
    }
}

/// The note for one file. One other agent:
/// `[AgentMux] Agent4 also has uncommitted changes in crates/srv/src/main.rs (repo o/r,
/// branch agent4/x). Message them before changing it.` or `... Agent5 also
/// edited crates/srv/src/main.rs 12 minutes ago (...)`. Several are listed in one note.
pub fn note_text(file: &EditedFile, overlaps: &[Overlap], now_ms: u64) -> String {
    if let [o] = overlaps {
        let what = match (o.kind, o.edited_ms) {
            (OverlapKind::Dirty, _) => format!("has uncommitted changes in {}", file.rel),
            (OverlapKind::Edited, Some(ts)) => format!("edited {} {}", file.rel, ago(now_ms, ts)),
            (OverlapKind::Edited, None) => format!("recently edited {}", file.rel),
        };
        let branch = o.branch.as_deref().map(|b| format!(", branch {b}")).unwrap_or_default();
        return format!(
            "[AgentMux] {} also {what} (repo {}{branch}). Message them before changing it.",
            o.agent, file.repo
        );
    }
    let who: Vec<String> = overlaps
        .iter()
        .map(|o| {
            let branch = o.branch.as_deref().map(|b| format!(" (branch {b})")).unwrap_or_default();
            format!("{} {}{branch}", o.agent, doing(o, now_ms))
        })
        .collect();
    format!(
        "[AgentMux] Other agents are also working on {} (repo {}): {}. Message them before changing it.",
        file.rel,
        file.repo,
        who.join("; ")
    )
}

/// The note that ends an agent's window: this file's overlap, how many
/// files overlapped in the window, and that nothing more comes until it ends.
pub fn summary_text(file: &EditedFile, overlaps: &[Overlap], count: usize, quiet_ms: u64, now_ms: u64) -> String {
    let who: Vec<String> = overlaps.iter().map(|o| format!("{} {}", o.agent, doing(o, now_ms))).collect();
    let minutes = quiet_ms.div_ceil(60_000).max(1);
    format!(
        "[AgentMux] {count} files you edited in the last {} minutes are also being changed by other agents; \
         the latest is {} (repo {}; {}). No more of these notes for {minutes} minute{}: before changing a file, \
         call WhoIsWorkingOn with its path, and message whoever it names.",
        WINDOW_MS / 60_000,
        file.rel,
        file.repo,
        who.join(", "),
        if minutes == 1 { "" } else { "s" },
    )
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start_ms: u64,
    sent: usize,
    summarized: bool,
}

/// What to tell an agent about one overlapping file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Note(String),
    Summary(String),
}

/// Remembers what each agent has been told, so notes stay rare: one per
/// (agent, other agent, repository, path) per [`PAIR_QUIET_MS`], at most
/// [`WINDOW_NOTES`] per agent per [`WINDOW_MS`], then one summary and quiet
/// until that window ends. Agents are keyed by name, case-insensitively.
#[derive(Debug, Default)]
pub struct RateLimiter {
    pairs: HashMap<(String, String, String, String), u64>,
    windows: HashMap<String, Window>,
    checked: HashMap<(String, String), u64>,
}

impl RateLimiter {
    fn prune(&mut self, now_ms: u64) {
        self.pairs.retain(|_, t| now_ms.saturating_sub(*t) < PAIR_QUIET_MS);
        self.windows.retain(|_, w| now_ms.saturating_sub(w.start_ms) < WINDOW_MS);
        self.checked.retain(|_, t| now_ms.saturating_sub(*t) < RECHECK_MS);
    }

    /// Whether `block`'s edit of `path` should be checked now: once per
    /// [`RECHECK_MS`] per (block, file), so an agent editing one file over
    /// and over costs one check.
    pub fn should_check(&mut self, block: &str, path: &str, now_ms: u64) -> bool {
        self.prune(now_ms);
        let key = (block.to_string(), normalize_path(path));
        if self.checked.contains_key(&key) {
            return false;
        }
        self.checked.insert(key, now_ms);
        true
    }

    /// The note (if any) to deliver to `me` about `overlaps` on `file`, and
    /// the record that it was sent. Overlaps this pair was already told
    /// about are dropped first; with none left, nothing is sent.
    pub fn decide(&mut self, me: &str, file: &EditedFile, overlaps: &[Overlap], now_ms: u64) -> Option<Decision> {
        self.prune(now_ms);
        let me_key = me.to_lowercase();
        let pair = |o: &Overlap| (me_key.clone(), o.agent.to_lowercase(), file.repo_key.clone(), file.rel.clone());
        let fresh: Vec<Overlap> = overlaps.iter().filter(|o| !self.pairs.contains_key(&pair(o))).cloned().collect();
        if fresh.is_empty() {
            return None;
        }
        let w = self.windows.entry(me_key.clone()).or_insert(Window { start_ms: now_ms, sent: 0, summarized: false });
        let decision = if w.summarized {
            return None;
        } else if w.sent < WINDOW_NOTES {
            w.sent += 1;
            Decision::Note(note_text(file, &fresh, now_ms))
        } else {
            w.summarized = true;
            let quiet = (w.start_ms + WINDOW_MS).saturating_sub(now_ms);
            Decision::Summary(summary_text(file, &fresh, w.sent + 1, quiet, now_ms))
        };
        for o in &fresh {
            self.pairs.insert(pair(o), now_ms);
        }
        Some(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::work_facts::FactFile;

    const NOW: u64 = 10 * 60 * 60 * 1000;
    const MIN: u64 = 60_000;

    fn agent(name: &str, repo: &str, root: &str, branch: &str) -> WorkFacts {
        WorkFacts {
            agent: name.into(),
            block_id: format!("block-{name}-{root}"),
            channel: "stable".into(),
            repo: Some(repo.into()),
            repo_root: Some(root.into()),
            branch: Some(branch.into()),
            ..Default::default()
        }
    }

    fn edited(root: &str, rel: &str, ts: u64) -> FactFile {
        FactFile { path: format!("{root}/{rel}"), rel: Some(rel.into()), tool: "Edit".into(), ts_ms: ts }
    }

    fn file(rel: &str) -> EditedFile {
        EditedFile { repo_key: "o/r".into(), repo: "o/r".into(), rel: rel.into() }
    }

    fn dirty(name: &str) -> Overlap {
        Overlap {
            agent: name.into(),
            kind: OverlapKind::Dirty,
            branch: Some(format!("{}/x", name.to_lowercase())),
            edited_ms: None,
        }
    }

    #[test]
    fn an_edit_is_placed_by_repository_relative_path_across_clones() {
        let me = agent("Me", "o/r", "C:/a/me/r", "main");
        let other = agent("Other", "o/r", "C:/a/other/clone", "feat");
        let all = vec![me.clone(), other];
        assert_eq!(
            locate(r"C:\a\me\r\crates\srv\lib.rs", Some(&me), &all),
            Some(EditedFile { repo_key: "o/r".into(), repo: "o/r".into(), rel: "crates/srv/lib.rs".into() })
        );
        // Editing a file in someone else's clone still places it in o/r.
        assert_eq!(locate("C:/a/other/clone/x.rs", Some(&me), &all).map(|f| f.rel), Some("x.rs".into()));
        assert_eq!(locate("D:/nowhere/x.rs", Some(&me), &all), None, "in no known repository");
    }

    #[test]
    fn the_same_file_in_another_clone_matches_and_other_repositories_dont() {
        let mut b = agent("Agent4", "o/r", "/b", "agent4/x");
        b.dirty_files = vec!["crates/srv/lib.rs".into()];
        let mut c = agent("Agent5", "o/r", "/c", "agent5/y");
        c.recent_files = vec![edited("/c", "crates/srv/lib.rs", NOW - 5 * MIN)];
        let mut stranger = agent("S", "o/other", "/s", "main");
        stranger.dirty_files = vec!["crates/srv/lib.rs".into()];
        let mut elsewhere = agent("E", "o/r", "/e", "main");
        elsewhere.dirty_files = vec!["crates/srv/other.rs".into()];

        let got = overlaps(&file("crates/srv/lib.rs"), "Me", &[b, c, stranger, elsewhere], NOW);
        let names: Vec<(&str, OverlapKind)> = got.iter().map(|o| (o.agent.as_str(), o.kind)).collect();
        assert_eq!(names, vec![("Agent4", OverlapKind::Dirty), ("Agent5", OverlapKind::Edited)]);
        assert_eq!(got[1].edited_ms, Some(NOW - 5 * MIN));
    }

    #[test]
    fn an_untracked_folder_covers_the_files_inside_it() {
        let mut b = agent("B", "o/r", "/b", "x");
        b.dirty_files = vec!["notes/new/".into()];
        assert_eq!(overlaps(&file("notes/new/a.md"), "Me", &[b], NOW).len(), 1);
    }

    #[test]
    fn the_editor_itself_is_never_named_in_any_clone_or_channel() {
        let mut own_clone = agent("me", "o/r", "/me2", "other-branch");
        own_clone.dirty_files = vec!["a.rs".into()];
        let mut own_dev = agent("ME", "o/r", "/me3", "x");
        own_dev.channel = "dev".into();
        own_dev.recent_files = vec![edited("/me3", "a.rs", NOW)];
        assert!(overlaps(&file("a.rs"), "Me", &[own_clone, own_dev], NOW).is_empty());
    }

    #[test]
    fn edits_older_than_two_hours_dont_count_but_uncommitted_changes_do() {
        let mut old = agent("Old", "o/r", "/o", "x");
        old.recent_files = vec![edited("/o", "a.rs", NOW - EDITED_WINDOW_MS - 1)];
        let mut edge = agent("Edge", "o/r", "/e", "x");
        edge.recent_files = vec![edited("/e", "a.rs", NOW - EDITED_WINDOW_MS)];
        let mut dirty_old = agent("DirtyOld", "o/r", "/d", "x");
        dirty_old.dirty_files = vec!["a.rs".into()];
        dirty_old.recent_files = vec![edited("/d", "a.rs", NOW - 5 * EDITED_WINDOW_MS)];
        let got = overlaps(&file("a.rs"), "Me", &[old, edge, dirty_old], NOW);
        let names: Vec<&str> = got.iter().map(|o| o.agent.as_str()).collect();
        assert_eq!(names, vec!["DirtyOld", "Edge"]);
        assert_eq!(got[0].edited_ms, None, "an edit outside the window says nothing about when");
    }

    #[test]
    fn one_agent_in_two_panes_is_named_once_with_its_strongest_overlap() {
        let mut a1 = agent("Agent4", "o/r", "/a1", "x");
        a1.recent_files = vec![edited("/a1", "a.rs", NOW - MIN)];
        let mut a2 = agent("agent4", "o/r", "/a2", "y");
        a2.dirty_files = vec!["a.rs".into()];
        let got = overlaps(&file("a.rs"), "Me", &[a1, a2], NOW);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, OverlapKind::Dirty);
    }

    #[test]
    fn the_note_names_the_agent_file_repository_and_branch() {
        let f = EditedFile {
            repo_key: "agentmuxai/agentmux".into(),
            repo: "agentmuxai/agentmux".into(),
            rel: "crates/srv/src/muxbus/presence.rs".into(),
        };
        assert_eq!(
            note_text(&f, &[dirty("Agent4")], NOW),
            "[AgentMux] Agent4 also has uncommitted changes in crates/srv/src/muxbus/presence.rs \
             (repo agentmuxai/agentmux, branch agent4/x). Message them before changing it."
        );
        let e = Overlap {
            agent: "Agent5".into(),
            kind: OverlapKind::Edited,
            branch: Some("agent5/y".into()),
            edited_ms: Some(NOW - 12 * MIN),
        };
        assert_eq!(
            note_text(&f, std::slice::from_ref(&e), NOW),
            "[AgentMux] Agent5 also edited crates/srv/src/muxbus/presence.rs 12 minutes ago \
             (repo agentmuxai/agentmux, branch agent5/y). Message them before changing it."
        );
        let no_branch = Overlap { branch: None, edited_ms: Some(NOW), ..e.clone() };
        assert!(note_text(&f, &[no_branch], NOW).contains("just now (repo agentmuxai/agentmux)."));
        assert_eq!(
            note_text(&f, &[dirty("Agent4"), e], NOW),
            "[AgentMux] Other agents are also working on crates/srv/src/muxbus/presence.rs (repo agentmuxai/agentmux): \
             Agent4 has uncommitted changes in it (branch agent4/x); Agent5 edited it 12 minutes ago (branch agent5/y). \
             Message them before changing it."
        );
    }

    #[test]
    fn one_note_per_pair_per_two_hours() {
        let mut rl = RateLimiter::default();
        assert!(matches!(rl.decide("Me", &file("a.rs"), &[dirty("B")], NOW), Some(Decision::Note(_))));
        assert_eq!(rl.decide("me", &file("a.rs"), &[dirty("b")], NOW + MIN), None, "same pair, any case");
        // A new agent on the same file is new news; only it is named.
        match rl.decide("Me", &file("a.rs"), &[dirty("B"), dirty("C")], NOW + 2 * MIN) {
            Some(Decision::Note(t)) => assert!(t.starts_with("[AgentMux] C also") && !t.contains(" B "), "{t}"),
            other => panic!("{other:?}"),
        }
        // Another agent being told about the same pair is separate.
        assert!(rl.decide("Other", &file("a.rs"), &[dirty("B")], NOW + 3 * MIN).is_some());
        assert!(rl.decide("Me", &file("a.rs"), &[dirty("B")], NOW + PAIR_QUIET_MS - 1).is_none());
        assert!(rl.decide("Me", &file("a.rs"), &[dirty("B")], NOW + PAIR_QUIET_MS).is_some(), "after two hours");
    }

    #[test]
    fn three_notes_per_ten_minutes_then_one_summary_then_quiet() {
        let mut rl = RateLimiter::default();
        for (i, f) in ["a.rs", "b.rs", "c.rs"].iter().enumerate() {
            assert!(
                matches!(rl.decide("Me", &file(f), &[dirty("B")], NOW + i as u64 * MIN), Some(Decision::Note(_))),
                "{f}"
            );
        }
        match rl.decide("Me", &file("d.rs"), &[dirty("B")], NOW + 4 * MIN) {
            Some(Decision::Summary(t)) => {
                assert!(t.starts_with("[AgentMux] 4 files you edited in the last 10 minutes"), "{t}");
                assert!(t.contains("the latest is d.rs (repo o/r; B has uncommitted changes in it)"), "{t}");
                assert!(t.contains("No more of these notes for 6 minutes"), "{t}");
                assert!(t.contains("WhoIsWorkingOn"), "{t}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rl.decide("Me", &file("e.rs"), &[dirty("B")], NOW + 5 * MIN), None, "quiet for the window");
        assert!(rl.decide("You", &file("e.rs"), &[dirty("B")], NOW + 5 * MIN).is_some(), "per agent");
        // The window ends ten minutes after its first note; e.rs was never
        // told, so it is news now.
        assert!(matches!(rl.decide("Me", &file("e.rs"), &[dirty("B")], NOW + WINDOW_MS), Some(Decision::Note(_))));
    }

    #[test]
    fn the_same_file_is_checked_once_a_minute_per_pane() {
        let mut rl = RateLimiter::default();
        assert!(rl.should_check("b1", r"C:\r\a.rs", NOW));
        assert!(!rl.should_check("b1", "C:/r/a.rs", NOW + 1000));
        assert!(rl.should_check("b2", "C:/r/a.rs", NOW + 1000));
        assert!(rl.should_check("b1", "C:/r/a.rs", NOW + RECHECK_MS));
    }
}
