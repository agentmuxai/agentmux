// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `WhoIsWorkingOn`: which other agents' work facts touch a path,
//! repository, branch or topic
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.2). Pure, so
//! every rule is tested without a running srv.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::paths::{clean_relative, is_absolute, normalize_path, normalize_repo, path_matches, relative_to};
use super::WorkFacts;

/// What the caller asked about. Every field is optional; with none, the
/// caller's own repository is the question.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WhoQuery {
    pub path: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub query: Option<String>,
}

/// The question after resolving it against the caller's own facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Target {
    /// Repository key (`owner/repo`, or `local:<folder>` without a remote).
    pub repo: Option<String>,
    /// Repository-relative path or folder; `""` is the whole repository.
    pub path: Option<String>,
    /// An absolute path that is in no known clone: compared as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_path: Option<String>,
    pub branch: Option<String>,
    pub query: Option<String>,
}

/// How one agent's work touches the question. Ordered by strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    /// The file has uncommitted changes in that agent's clone.
    Dirty,
    /// That agent's edit tools changed the file recently.
    Edited,
    /// Same repository and branch.
    Branch,
    /// Same repository (asked about the repository as a whole).
    Repo,
    Goal,
    Todo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhoMatch {
    pub kind: MatchKind,
    pub detail: String,
    /// When the matched activity happened (Unix ms), where known.
    pub since_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhoResult {
    pub agent: String,
    pub channel: String,
    pub status: String,
    pub goal: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub matches: Vec<WhoMatch>,
}

/// How many file names one match's detail lists before "+N more".
const DETAIL_FILES: usize = 5;

/// Resolve `q` against the caller's facts (`caller`) and every known clone
/// (`all`): a path absolute in some clone becomes (repository, relative
/// path), and the caller's repository is assumed when none is named.
pub fn resolve(q: &WhoQuery, caller: Option<&WorkFacts>, all: &[WorkFacts]) -> Result<Target, String> {
    let nonempty = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let mut t = Target { branch: nonempty(&q.branch), query: nonempty(&q.query), ..Default::default() };
    let named_repo = match nonempty(&q.repo) {
        Some(r) => Some(normalize_repo(&r).ok_or_else(|| format!("repo {r:?} is not owner/repo or a remote URL"))?),
        None => None,
    };
    let caller_repo = caller.and_then(WorkFacts::repo_key);

    if let Some(raw) = nonempty(&q.path) {
        if is_absolute(&raw) {
            let abs = normalize_path(&raw);
            // The caller's own clone first, then any other agent's.
            let owner = caller.into_iter().chain(all.iter()).find_map(|f| {
                let rel = relative_to(&abs, f.repo_root.as_deref()?)?;
                Some((f.repo_key()?, rel))
            });
            match owner {
                Some((repo, rel)) => {
                    t.repo = named_repo.clone().or(Some(repo));
                    t.path = Some(rel);
                }
                None => t.absolute_path = Some(abs),
            }
        } else {
            t.path = Some(clean_relative(&raw));
        }
    }
    if t.repo.is_none() {
        t.repo = named_repo.or(caller_repo);
    }
    if t.path.is_some() && t.repo.is_none() {
        return Err("path is relative to a repository, but no repository is known: pass `repo`, \
                    an absolute path, or call this from inside a repository"
            .to_string());
    }
    if t.path.is_none() && t.absolute_path.is_none() && t.branch.is_none() && t.query.is_none() && t.repo.is_none() {
        return Err("nothing to look for: pass path, repo, branch or query (or call this from inside a \
                    repository to ask about it)"
            .to_string());
    }
    Ok(t)
}

/// Lowercase alphanumeric words.
fn words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Every word of the query appears in `text`, case-insensitively.
fn mentions(query_words: &HashSet<String>, text: &str) -> bool {
    !query_words.is_empty() && query_words.is_subset(&words(text))
}

/// The first few of `files`, then "(+N more)" up to `total`.
fn list_files(files: &[String], total: usize) -> String {
    let mut s = files.iter().take(DETAIL_FILES).cloned().collect::<Vec<_>>().join(", ");
    let shown = files.len().min(DETAIL_FILES);
    if total > shown {
        s.push_str(&format!(" (+{} more)", total - shown));
    }
    s
}

/// What `other`'s facts say about `t`, strongest first.
fn matches_for(t: &Target, other: &WorkFacts) -> Vec<WhoMatch> {
    let mut out = Vec::new();
    let same_repo = t.repo.is_some() && other.repo_key() == t.repo;
    let repo_name = other.repo.clone().or_else(|| other.repo_root.clone()).unwrap_or_default();

    // Files: repository-relative within the same repository, else (a path in
    // no known clone) the absolute path as written.
    let file_target: Option<(&str, bool)> = match (&t.path, &t.absolute_path) {
        (Some(rel), _) if same_repo => Some((rel.as_str(), false)),
        (None, Some(abs)) => Some((abs.as_str(), true)),
        _ => None,
    };
    let repo_only = t.path.is_none() && t.absolute_path.is_none() && t.branch.is_none() && t.query.is_none();
    if let Some((target, absolute)) = file_target.or(if repo_only && same_repo { Some(("", false)) } else { None }) {
        let dirty: Vec<String> = other
            .dirty_files
            .iter()
            .filter(|f| {
                if absolute {
                    let abs = format!("{}/{}", other.repo_root.as_deref().unwrap_or(""), f);
                    path_matches(&normalize_path(&abs), target)
                } else {
                    path_matches(f, target)
                }
            })
            .cloned()
            .collect();
        let edited: Vec<&super::FactFile> = other
            .recent_files
            .iter()
            .filter(|r| {
                if absolute {
                    path_matches(&r.path, target)
                } else {
                    same_repo && r.rel.as_deref().is_some_and(|rel| path_matches(rel, target))
                }
            })
            .collect();
        let last_edit = |file: &str| {
            edited.iter().filter(|r| r.rel.as_deref() == Some(file) || r.path == file).map(|r| r.ts_ms).max()
        };
        if !dirty.is_empty() {
            // The whole repository has `dirty_count` files even past the
            // stored list; a narrower target counts what matched.
            let total = if target.is_empty() && !absolute { other.dirty_count.max(dirty.len()) } else { dirty.len() };
            out.push(WhoMatch {
                kind: MatchKind::Dirty,
                detail: format!("uncommitted in {repo_name}: {}", list_files(&dirty, total)),
                since_ms: dirty.iter().filter_map(|f| last_edit(f)).max(),
            });
        }
        if !edited.is_empty() {
            let names: Vec<String> =
                edited.iter().map(|r| format!("{} ({})", r.rel.as_deref().unwrap_or(&r.path), r.tool)).collect();
            out.push(WhoMatch {
                kind: MatchKind::Edited,
                detail: format!("recently edited: {}", list_files(&names, names.len())),
                since_ms: edited.iter().map(|r| r.ts_ms).max(),
            });
        }
        if repo_only && out.is_empty() {
            out.push(WhoMatch {
                kind: MatchKind::Repo,
                detail: match &other.branch {
                    Some(b) => format!("working in {repo_name} on branch {b}"),
                    None => format!("working in {repo_name}"),
                },
                since_ms: None,
            });
        }
    }

    if let Some(branch) = &t.branch {
        let repo_ok = t.repo.is_none() || same_repo;
        if repo_ok && other.branch.as_deref() == Some(branch.as_str()) {
            out.push(WhoMatch {
                kind: MatchKind::Branch,
                detail: format!("on branch {branch} in {repo_name}"),
                since_ms: None,
            });
        }
    }

    if let Some(q) = &t.query {
        let qw = words(q);
        if let Some(goal) = other.goal.as_deref().filter(|g| mentions(&qw, g)) {
            out.push(WhoMatch { kind: MatchKind::Goal, detail: goal.to_string(), since_ms: None });
        }
        for todo in other.todos.iter().filter(|td| mentions(&qw, &td.text)).take(3) {
            out.push(WhoMatch {
                kind: MatchKind::Todo,
                detail: format!("[{}] {}", todo.status, todo.text),
                since_ms: None,
            });
        }
    }

    out.sort_by_key(|m| m.kind);
    out
}

/// Every agent in `others` (the caller already left out) whose work touches
/// `t`, strongest match first, then most recent.
pub fn who_is_working_on(t: &Target, others: &[WorkFacts]) -> Vec<WhoResult> {
    let mut results: Vec<WhoResult> = others
        .iter()
        .filter_map(|f| {
            let matches = matches_for(t, f);
            (!matches.is_empty()).then(|| WhoResult {
                agent: f.agent.clone(),
                channel: f.channel.clone(),
                status: f.status.clone(),
                goal: f.goal.clone(),
                repo: f.repo.clone(),
                branch: f.branch.clone(),
                matches,
            })
        })
        .collect();
    results.sort_by(|a, b| {
        let strongest = |r: &WhoResult| r.matches.first().map(|m| m.kind);
        let newest = |r: &WhoResult| r.matches.iter().filter_map(|m| m.since_ms).max();
        strongest(a)
            .cmp(&strongest(b))
            .then_with(|| newest(b).cmp(&newest(a)))
            .then_with(|| a.agent.cmp(&b.agent))
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::reactive::progress_watcher::TodoItem;
    use crate::backend::work_facts::FactFile;

    /// An agent with its own clone of `repo` at `root`.
    fn agent(name: &str, repo: &str, root: &str, branch: &str) -> WorkFacts {
        WorkFacts {
            agent: name.into(),
            block_id: format!("block-{name}"),
            channel: "stable".into(),
            status: "busy".into(),
            repo: Some(repo.into()),
            repo_root: Some(root.into()),
            branch: Some(branch.into()),
            ..Default::default()
        }
    }

    fn edited(root: &str, rel: &str, ts: u64) -> FactFile {
        FactFile { path: format!("{root}/{rel}"), rel: Some(rel.into()), tool: "Edit".into(), ts_ms: ts }
    }

    fn q(path: Option<&str>, repo: Option<&str>, branch: Option<&str>, query: Option<&str>) -> WhoQuery {
        WhoQuery {
            path: path.map(Into::into),
            repo: repo.map(Into::into),
            branch: branch.map(Into::into),
            query: query.map(Into::into),
        }
    }

    fn ask(query: WhoQuery, caller: &WorkFacts, others: &[WorkFacts]) -> Vec<WhoResult> {
        let mut all = vec![caller.clone()];
        all.extend_from_slice(others);
        let t = resolve(&query, Some(caller), &all).unwrap();
        who_is_working_on(&t, others)
    }

    /// Two clones of the same remote in different folders match on the
    /// repository-relative path, whichever spelling the caller used.
    #[test]
    fn clones_in_different_folders_match_on_repository_relative_paths() {
        let me = agent("Me", "o/r", "C:/a/me/r", "main");
        let mut other = agent("Other", "o/r", "C:/a/other/clone", "feat");
        other.dirty_files = vec!["crates/srv/lib.rs".into()];
        other.dirty_count = 1;
        other.recent_files = vec![edited("C:/a/other/clone", "crates/srv/lib.rs", 50)];

        for path in ["C:/a/me/r/crates/srv/lib.rs", r"C:\a\me\r\crates\srv\lib.rs", "crates/srv/lib.rs"] {
            let r = ask(q(Some(path), None, None, None), &me, std::slice::from_ref(&other));
            assert_eq!(r.len(), 1, "{path}");
            assert_eq!(r[0].agent, "Other");
            let kinds: Vec<MatchKind> = r[0].matches.iter().map(|m| m.kind).collect();
            assert_eq!(kinds, vec![MatchKind::Dirty, MatchKind::Edited]);
            assert_eq!(r[0].matches[0].since_ms, Some(50));
            assert!(r[0].matches[0].detail.contains("crates/srv/lib.rs"));
        }

        // A different repository with the same relative path is not a match.
        let stranger = WorkFacts { dirty_files: vec!["crates/srv/lib.rs".into()], ..agent("S", "o/other", "C:/s", "main") };
        assert!(ask(q(Some("crates/srv/lib.rs"), None, None, None), &me, &[stranger]).is_empty());
    }

    #[test]
    fn a_folder_matches_files_inside_it_by_prefix() {
        let me = agent("Me", "o/r", "/me", "main");
        let mut other = agent("Other", "o/r", "/other", "feat");
        other.dirty_files = vec!["crates/srv/src/a.rs".into(), "docs/x.md".into()];
        other.recent_files = vec![edited("/other", "crates/srv/src/b.rs", 9)];

        let r = ask(q(Some("crates/srv"), None, None, None), &me, std::slice::from_ref(&other));
        assert_eq!(r.len(), 1);
        assert!(r[0].matches[0].detail.contains("crates/srv/src/a.rs"));
        assert!(!r[0].matches[0].detail.contains("docs/x.md"));
        assert!(r[0].matches[1].detail.contains("crates/srv/src/b.rs"));

        assert!(ask(q(Some("crates/sr"), None, None, None), &me, &[other]).is_empty(), "a name prefix isn't a folder");
    }

    #[test]
    fn query_matches_words_in_goals_and_todos_case_insensitively() {
        let me = agent("Me", "o/r", "/me", "main");
        let mut a = agent("A", "o/x", "/a", "main");
        a.goal = Some("Add the MuxBus allowlist for presence".into());
        let mut b = agent("B", "o/y", "/b", "main");
        b.todos = vec![TodoItem { text: "Wire the muxbus Allowlist check".into(), status: "in_progress".into() }];
        let mut c = agent("C", "o/z", "/c", "main");
        c.goal = Some("Allowlists everywhere".into());

        let r = ask(q(None, None, None, Some("muxbus ALLOWLIST")), &me, &[a, b, c]);
        let got: Vec<(&str, MatchKind)> = r.iter().map(|r| (r.agent.as_str(), r.matches[0].kind)).collect();
        assert_eq!(got, vec![("A", MatchKind::Goal), ("B", MatchKind::Todo)], "whole words, any order, any case");
        assert_eq!(r[1].matches[0].detail, "[in_progress] Wire the muxbus Allowlist check");
    }

    #[test]
    fn branch_matches_within_the_repository() {
        let me = agent("Me", "o/r", "/me", "main");
        let a = agent("A", "o/r", "/a", "agenty/x");
        let b = agent("B", "o/other", "/b", "agenty/x");
        let r = ask(q(None, None, Some("agenty/x"), None), &me, &[a, b]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].agent, "A");
        assert_eq!(r[0].matches[0].kind, MatchKind::Branch);
    }

    #[test]
    fn with_nothing_named_the_callers_repository_is_the_question() {
        let me = agent("Me", "o/r", "/me", "main");
        let mut a = agent("A", "o/r", "/a", "feat");
        a.dirty_files = vec!["x.rs".into()];
        a.dirty_count = 12;
        let b = agent("B", "o/r", "/b", "fix");
        let c = agent("C", "o/else", "/c", "main");
        let r = ask(WhoQuery::default(), &me, &[b, a, c]);
        let got: Vec<(&str, MatchKind)> = r.iter().map(|r| (r.agent.as_str(), r.matches[0].kind)).collect();
        assert_eq!(got, vec![("A", MatchKind::Dirty), ("B", MatchKind::Repo)], "ordered by strength");
        assert!(r[0].matches[0].detail.contains("x.rs (+11 more)"), "{}", r[0].matches[0].detail);
        assert_eq!(r[1].matches[0].detail, "working in o/r on branch fix");
    }

    #[test]
    fn an_explicit_repo_wins_and_bad_input_is_explained() {
        let me = agent("Me", "o/r", "/me", "main");
        let mut a = agent("A", "agentmuxai/agentmux", "/a", "feat");
        a.dirty_files = vec!["src/x.rs".into()];
        let r = ask(q(Some("src/x.rs"), Some("https://github.com/AgentMuxAI/agentmux.git"), None, None), &me, &[a]);
        assert_eq!(r.len(), 1);

        assert!(resolve(&q(Some("a.rs"), None, None, None), None, &[]).is_err(), "relative path, no repository");
        assert!(resolve(&WhoQuery::default(), None, &[]).is_err(), "nothing asked");
        assert!(resolve(&q(None, Some("not a repo"), None, None), None, &[]).is_err());
    }

    #[test]
    fn an_absolute_path_in_another_agents_clone_resolves_through_it() {
        let me = WorkFacts { agent: "Me".into(), ..Default::default() };
        let mut a = agent("A", "o/r", "/a/clone", "feat");
        a.dirty_files = vec!["lib.rs".into()];
        let b = WorkFacts { dirty_files: vec!["lib.rs".into()], ..agent("B", "o/r", "/b/clone", "main") };
        let r = ask(q(Some("/a/clone/lib.rs"), None, None, None), &me, &[a, b]);
        let names: Vec<&str> = r.iter().map(|r| r.agent.as_str()).collect();
        assert_eq!(names, vec!["A", "B"], "both clones of o/r have lib.rs uncommitted");
    }

    #[test]
    fn a_path_in_no_known_clone_is_compared_as_written() {
        let me = WorkFacts { agent: "Me".into(), ..Default::default() };
        let other = WorkFacts {
            agent: "O".into(),
            recent_files: vec![FactFile { path: "/tmp/notes/a.md".into(), rel: None, tool: "Write".into(), ts_ms: 3 }],
            ..Default::default()
        };
        let r = ask(q(Some("/tmp/notes"), None, None, None), &me, &[other]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].matches[0].kind, MatchKind::Edited);
    }

    #[test]
    fn results_order_by_strength_then_recency() {
        let me = agent("Me", "o/r", "/me", "main");
        let mut old = agent("Old", "o/r", "/old", "x");
        old.recent_files = vec![edited("/old", "a.rs", 10)];
        let mut new = agent("New", "o/r", "/new", "y");
        new.recent_files = vec![edited("/new", "a.rs", 20)];
        let mut dirty = agent("Dirty", "o/r", "/d", "z");
        dirty.dirty_files = vec!["a.rs".into()];
        let r = ask(q(Some("a.rs"), None, None, None), &me, &[old, new, dirty]);
        let names: Vec<&str> = r.iter().map(|r| r.agent.as_str()).collect();
        assert_eq!(names, vec!["Dirty", "New", "Old"]);
    }
}
