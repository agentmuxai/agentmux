// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `fs.git_status`: what git says about the entries of a listed folder, for
//! the Files pane's markers (modified, untracked, ignored...) and its status
//! line (branch, ahead/behind).
//!
//! Running git inside a folder the user merely browsed must not run code the
//! folder chose. A repository's own config can name programs that `git
//! status` executes: `core.fsmonitor`, and a `filter.<name>.clean` (or
//! `.process`) applied through its `.gitattributes` whenever a file's stat
//! data differs from the index. Command-line `-c` settings win over every
//! config file, so the fsmonitor is turned off and every filter the
//! repository itself defines is blanked; the user's own system and global
//! filters (Git LFS) are left alone. Submodules, which have configs of their
//! own, are not entered. The status never writes the index
//! (`--no-optional-locks`), prompts for nothing, and is killed after a few
//! seconds on a huge repository.
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §12 (Phase 2, git
//! decorations).

use std::collections::HashMap;
use std::time::Duration;

use agentmux_common::win32::NoWindow;

use crate::backend::rpc_types::{FsGitEntry, FsGitState, FsGitStatus};

use super::platform::display_path;

/// A status that takes longer than this is abandoned: the pane shows no
/// markers rather than waiting.
const GIT_TIMEOUT: Duration = Duration::from_secs(4);

/// Settings forced on every git run here, ahead of any config file.
const SAFE_CONFIG: &[&str] = &[
    // A repository's config could name a program to run (see module docs).
    "-c",
    "core.fsmonitor=false",
    // Don't write the untracked cache into the repository.
    "-c",
    "core.untrackedCache=false",
];

/// `fs.git_status` for `raw` (a folder the pane listed).
pub async fn git_status(raw: &str) -> FsGitStatus {
    let dir = match super::resolve_request_path(raw).and_then(|p| p.canonicalize().map_err(|e| e.to_string())) {
        Ok(d) if d.is_dir() => d,
        _ => return FsGitStatus::default(),
    };
    let dir_arg = display_path(&dir);

    // Where the folder sits in its repository ("" at the top). Failing here
    // is the ordinary "not a repository", or no git at all: no markers.
    let prefix = match run_git(&dir_arg, &["rev-parse", "--show-prefix"]).await {
        Ok(out) => String::from_utf8_lossy(&out).trim_end_matches(['\r', '\n']).to_string(),
        Err(_) => return FsGitStatus::default(),
    };
    // The repository's own filters, blanked for this run (see module docs).
    // Fails closed: if the config can't be read, git status doesn't run
    // (muxreview on #4223): unblanked filters would run.
    let Ok(filters) = repo_filters(&dir_arg).await else {
        return FsGitStatus {
            in_repo: true,
            error: Some("Couldn't read this repository's settings, so git markers are off here.".to_string()),
            ..Default::default()
        };
    };
    // `-c` splits at the first `=`, so a filter named `x=y` can't be
    // blanked that way (muxreview on #4223). Such a name is never needed:
    // don't run git there at all.
    if filters.iter().any(|n| n.contains('=')) {
        return FsGitStatus {
            in_repo: true,
            error: Some("This repository's settings can't be read safely, so git markers are off here.".to_string()),
            ..Default::default()
        };
    }
    let mut args: Vec<String> = Vec::new();
    for name in filters {
        for key in ["clean", "smudge", "process"] {
            args.push("-c".into());
            args.push(format!("filter.{name}.{key}="));
        }
        args.push("-c".into());
        args.push(format!("filter.{name}.required=false"));
    }
    args.extend(
        [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=normal",
            "--ignored=matching",
            "--ignore-submodules=all",
            "--",
            ".",
        ]
        .map(String::from),
    );
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = match run_git(&dir_arg, &arg_refs).await
    {
        Ok(out) => out,
        Err(e) => return FsGitStatus { in_repo: true, error: Some(e.message()), ..Default::default() },
    };
    summarize(&parse_porcelain_v2(&out), &prefix)
}

/// The names of the filters a repository's own config defines (its
/// `.git/config`, worktree config, and what they include): every
/// `filter.<name>.*` key whose scope isn't the user's system or global
/// config. An older git without `--show-scope` gets every filter blanked.
/// `Err` when the config couldn't be read: the caller must not run git
/// status then. Only `--get-regexp`'s exit 1 ("no match") means none.
async fn repo_filters(dir: &str) -> Result<Vec<String>, ()> {
    let scoped = run_git(dir, &["config", "--show-scope", "--name-only", "--get-regexp", r"^filter\."]).await;
    let (out, scoped) = match scoped {
        Ok(out) => (out, true),
        Err(GitError::Exit(1)) => return Ok(Vec::new()),
        // An older git without `--show-scope`: every filter, unscoped.
        Err(_) => match run_git(dir, &["config", "--name-only", "--get-regexp", r"^filter\."]).await {
            Ok(out) => (out, false),
            Err(GitError::Exit(1)) => return Ok(Vec::new()),
            Err(_) => return Err(()),
        },
    };
    Ok(filter_names(&String::from_utf8_lossy(&out), scoped))
}

/// Filter names from `git config [--show-scope] --name-only --get-regexp
/// ^filter\.` output; with scopes, only those outside system and global.
pub fn filter_names(out: &str, scoped: bool) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in out.lines() {
        let (scope, key) = if scoped {
            match line.split_once(|c: char| c.is_whitespace()) {
                Some((s, k)) => (s, k.trim()),
                None => continue,
            }
        } else {
            ("", line.trim())
        };
        if scoped && (scope == "system" || scope == "global") {
            continue;
        }
        // filter.<name>.<key>; a name may itself contain dots.
        let Some(rest) = key.strip_prefix("filter.") else { continue };
        let Some((name, _)) = rest.rsplit_once('.') else { continue };
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Run git in `dir` with the safe settings; its stdout, or a sentence.
async fn run_git(dir: &str, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(SAFE_CONFIG).arg("--no-optional-locks").args(args);
    cmd.env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .no_window();
    // git needs nothing of this instance's identity.
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    let child = cmd.spawn().map_err(|e| GitError::Failed(format!("Couldn't run git: {e}")))?;
    match tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output()).await {
        Err(_) => Err(GitError::Failed("git took too long to answer.".to_string())),
        Ok(Err(e)) => Err(GitError::Failed(format!("git failed: {e}"))),
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(out)) => match out.status.code() {
            Some(code) if out.stderr.is_empty() => Err(GitError::Exit(code)),
            _ => Err(GitError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string())),
        },
    }
}

#[derive(Debug)]
enum GitError {
    /// git exited with this code and said nothing.
    Exit(i32),
    Failed(String),
}

impl GitError {
    fn message(self) -> String {
        match self {
            GitError::Exit(code) => format!("git exited with status {code}."),
            GitError::Failed(m) => m,
        }
    }
}

/// One path git reported, relative to the repository's top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRecord {
    pub path: String,
    pub state: FsGitState,
}

/// What `git status --porcelain=v2 -z --branch` printed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Porcelain {
    pub branch: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub records: Vec<GitRecord>,
}

/// The state an ordinary (`1`) or rename/copy (`2`) record's XY code means:
/// the most pressing of the staged (X) and unstaged (Y) changes.
fn state_of_xy(xy: &str) -> FsGitState {
    let mut state = FsGitState::Added;
    let mut seen = false;
    for c in xy.chars() {
        let s = match c {
            'M' | 'T' => FsGitState::Modified,
            'D' => FsGitState::Deleted,
            'R' | 'C' => FsGitState::Renamed,
            'A' => FsGitState::Added,
            _ => continue,
        };
        state = if seen { state.max(s) } else { s };
        seen = true;
    }
    state
}

/// Parse porcelain v2 output (`-z`: NUL-terminated records; a rename's
/// original path follows it as its own NUL-terminated field).
pub fn parse_porcelain_v2(out: &[u8]) -> Porcelain {
    let mut p = Porcelain::default();
    let mut fields = out.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(line) = fields.next() {
        if line.is_empty() {
            continue;
        }
        let kind = line.as_bytes()[0];
        match kind {
            b'#' => {
                if let Some(head) = line.strip_prefix("# branch.head ") {
                    if head != "(detached)" {
                        p.branch = Some(head.to_string());
                    }
                } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
                    let mut it = ab.split(' ');
                    p.ahead = it.next().and_then(|a| a.trim_start_matches('+').parse().ok());
                    p.behind = it.next().and_then(|b| b.trim_start_matches('-').parse().ok());
                }
            }
            b'1' => {
                // 1 XY sub mH mI mW hH hI path
                let parts: Vec<&str> = line.splitn(9, ' ').collect();
                if parts.len() == 9 {
                    p.records.push(GitRecord { path: parts[8].to_string(), state: state_of_xy(parts[1]) });
                }
            }
            b'2' => {
                // 2 XY sub mH mI mW hH hI Xscore path, then the original path.
                let parts: Vec<&str> = line.splitn(10, ' ').collect();
                let _orig = fields.next();
                if parts.len() == 10 {
                    p.records.push(GitRecord { path: parts[9].to_string(), state: state_of_xy(parts[1]).max(FsGitState::Renamed) });
                }
            }
            b'u' => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let parts: Vec<&str> = line.splitn(11, ' ').collect();
                if parts.len() == 11 {
                    p.records.push(GitRecord { path: parts[10].to_string(), state: FsGitState::Conflicted });
                }
            }
            b'?' | b'!' => {
                if let Some(path) = line.get(2..) {
                    let state = if kind == b'?' { FsGitState::Untracked } else { FsGitState::Ignored };
                    p.records.push(GitRecord { path: path.to_string(), state });
                }
            }
            _ => {}
        }
    }
    p
}

/// Fold the records under `prefix` (the listed folder, relative to the
/// repository's top, `/`-separated with a trailing `/`, or empty) into one
/// state per direct child. A folder takes the most pressing state of what's
/// inside it, except that a folder is only "ignored" when git ignores the
/// folder itself.
pub fn summarize(p: &Porcelain, prefix: &str) -> FsGitStatus {
    let mut by_child: HashMap<String, FsGitState> = HashMap::new();
    let mut changes = 0u64;
    for r in &p.records {
        let Some(rel) = r.path.strip_prefix(prefix) else { continue };
        let rel = rel.trim_end_matches('/');
        if rel.is_empty() {
            continue;
        }
        if r.state != FsGitState::Ignored {
            changes += 1;
        }
        let (child, nested) = match rel.split_once('/') {
            Some((c, _)) => (c, true),
            None => (rel, false),
        };
        // Something ignored deep inside doesn't make its folder look ignored.
        if nested && r.state == FsGitState::Ignored {
            continue;
        }
        by_child.entry(child.to_string()).and_modify(|s| *s = (*s).max(r.state)).or_insert(r.state);
    }
    let mut entries: Vec<FsGitEntry> = by_child.into_iter().map(|(name, state)| FsGitEntry { name, state }).collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    FsGitStatus {
        in_repo: true,
        branch: p.branch.clone(),
        ahead: p.ahead,
        behind: p.behind,
        entries,
        changes,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(records: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for r in records {
            out.extend_from_slice(r.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_branch_ahead_and_behind() {
        let p = parse_porcelain_v2(&z(&["# branch.oid abc", "# branch.head main", "# branch.upstream origin/main", "# branch.ab +2 -5"]));
        assert_eq!(p.branch.as_deref(), Some("main"));
        assert_eq!((p.ahead, p.behind), (Some(2), Some(5)));
        let detached = parse_porcelain_v2(&z(&["# branch.head (detached)"]));
        assert_eq!(detached.branch, None);
    }

    #[test]
    fn parses_every_record_kind_including_spaces_and_renames() {
        let out = z(&[
            "1 .M N... 100644 100644 100644 aaa bbb src/a file.ts",
            "1 A. N... 000000 100644 100644 000 ccc new.ts",
            "1 D. N... 100644 000000 000000 ddd 000 gone.ts",
            "2 R. N... 100644 100644 100644 eee eee R100 docs/new name.md",
            "docs/old name.md",
            "u UU N... 100644 100644 100644 100644 a b c conflict.rs",
            "? notes.txt",
            "! target/",
        ]);
        let p = parse_porcelain_v2(&out);
        let got: Vec<(&str, FsGitState)> = p.records.iter().map(|r| (r.path.as_str(), r.state)).collect();
        assert_eq!(
            got,
            vec![
                ("src/a file.ts", FsGitState::Modified),
                ("new.ts", FsGitState::Added),
                ("gone.ts", FsGitState::Deleted),
                ("docs/new name.md", FsGitState::Renamed),
                ("conflict.rs", FsGitState::Conflicted),
                ("notes.txt", FsGitState::Untracked),
                ("target/", FsGitState::Ignored),
            ]
        );
    }

    #[test]
    fn a_staged_add_with_later_edits_reads_as_modified() {
        assert_eq!(state_of_xy("AM"), FsGitState::Modified);
        assert_eq!(state_of_xy("MD"), FsGitState::Modified);
        assert_eq!(state_of_xy("R."), FsGitState::Renamed);
    }

    #[test]
    fn folds_records_into_the_listed_folders_children() {
        let p = parse_porcelain_v2(&z(&[
            "# branch.head main",
            "1 .M N... 1 1 1 a b app/src/x.ts",
            "? app/src/new/",
            "1 .M N... 1 1 1 a b app/README.md",
            "! app/node_modules/",
            "! app/src/cache/",
            "? app/notes.txt",
            "1 .M N... 1 1 1 a b other/y.ts",
        ]));
        let s = summarize(&p, "app/");
        let got: Vec<(&str, FsGitState)> = s.entries.iter().map(|e| (e.name.as_str(), e.state)).collect();
        assert_eq!(
            got,
            vec![
                ("README.md", FsGitState::Modified),
                ("node_modules", FsGitState::Ignored),
                ("notes.txt", FsGitState::Untracked),
                // Modified beats untracked; the ignored cache inside doesn't count.
                ("src", FsGitState::Modified),
            ]
        );
        assert_eq!(s.changes, 4);
        assert_eq!(s.branch.as_deref(), Some("main"));
    }

    #[test]
    fn at_the_top_the_prefix_is_empty() {
        let p = parse_porcelain_v2(&z(&["? a.txt", "1 .M N... 1 1 1 a b dir/b.txt"]));
        let s = summarize(&p, "");
        assert_eq!(s.entries.len(), 2);
    }

    /// The real thing, when git is installed: a repo whose config asks for an
    /// fsmonitor program gets none run, and the markers come back.
    #[tokio::test]
    async fn runs_git_without_the_repositorys_fsmonitor() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.email=a@b", "-c", "user.name=a"])
                .args(args)
                .current_dir(dir.path())
                .no_window()
                .output()
        };
        if git(&["init", "-q"]).map(|o| !o.status.success()).unwrap_or(true) {
            return; // No git on this machine.
        }
        std::fs::write(dir.path().join("tracked.txt"), "a").unwrap();
        let _ = git(&["add", "."]);
        let _ = git(&["commit", "-qm", "x"]);
        std::fs::write(dir.path().join("tracked.txt"), "b").unwrap();
        std::fs::write(dir.path().join("new.txt"), "n").unwrap();
        // A program git would run for core.fsmonitor: it would leave a mark.
        let mark = dir.path().join("fsmonitor-ran");
        let _ = git(&["config", "core.fsmonitor", &format!("touch '{}'", mark.display())]);

        let s = git_status(&dir.path().to_string_lossy()).await;
        assert!(s.in_repo, "{s:?}");
        let get = |n: &str| s.entries.iter().find(|e| e.name == n).map(|e| e.state);
        assert_eq!(get("tracked.txt"), Some(FsGitState::Modified));
        assert_eq!(get("new.txt"), Some(FsGitState::Untracked));
        assert!(!mark.exists(), "the repository's fsmonitor program ran");
    }

    #[test]
    fn blanks_the_repositorys_filters_and_keeps_the_users() {
        let out = "system\tfilter.lfs.clean\nglobal\tfilter.lfs.process\nlocal\tfilter.evil.clean\nlocal\tfilter.evil.process\nworktree\tfilter.a.b.smudge\n";
        assert_eq!(filter_names(out, true), vec!["evil".to_string(), "a.b".to_string()]);
        // Without scopes (an older git), every filter.
        assert_eq!(filter_names("filter.lfs.clean\nfilter.x.process\n", false), vec!["lfs".to_string(), "x".to_string()]);
    }

    /// The other route git status has to a repository's code: a clean filter
    /// named in `.git/config` and applied by `.gitattributes`, run when a
    /// file's stat data differs from the index. muxreview on #4223.
    #[tokio::test]
    async fn runs_git_without_the_repositorys_clean_filter() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.email=a@b", "-c", "user.name=a"])
                .args(args)
                .current_dir(dir.path())
                .no_window()
                .output()
        };
        if git(&["init", "-q"]).map(|o| !o.status.success()).unwrap_or(true) {
            return; // No git on this machine.
        }
        std::fs::write(dir.path().join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(dir.path().join("t.txt"), "a\n").unwrap();
        let _ = git(&["add", "."]);
        let _ = git(&["commit", "-qm", "x"]);
        let mark = dir.path().join("filter-ran");
        let mark_arg = mark.display().to_string().replace('\\', "/");
        let _ = git(&["config", "filter.evil.clean", &format!("sh -c 'touch \"{mark_arg}\"; cat'")]);
        // New stat data, same content: git must look at the content, through
        // the filter if it were allowed to.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("t.txt"), "a\n").unwrap();

        let s = git_status(&dir.path().to_string_lossy()).await;
        assert!(s.in_repo && s.error.is_none(), "{s:?}");
        assert!(!mark.exists(), "the repository's clean filter ran");
    }

    /// `-c filter.x=y.clean=` would set `filter.x`, not blank `x=y`'s clean
    /// command: a repository with such a filter gets no git run at all.
    #[tokio::test]
    async fn refuses_a_filter_name_that_c_cannot_blank() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.email=a@b", "-c", "user.name=a"])
                .args(args)
                .current_dir(dir.path())
                .no_window()
                .output()
        };
        if git(&["init", "-q"]).map(|o| !o.status.success()).unwrap_or(true) {
            return; // No git on this machine.
        }
        std::fs::write(dir.path().join(".gitattributes"), "*.txt filter=x=y\n").unwrap();
        std::fs::write(dir.path().join("t.txt"), "a\n").unwrap();
        let _ = git(&["add", "."]);
        let _ = git(&["commit", "-qm", "x"]);
        let mark = dir.path().join("filter-ran");
        let mark_arg = mark.display().to_string().replace('\\', "/");
        let _ = git(&["config", "filter.x=y.clean", &format!("sh -c 'touch \"{mark_arg}\"; cat'")]);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.path().join("t.txt"), "a\n").unwrap();

        let s = git_status(&dir.path().to_string_lossy()).await;
        assert!(s.entries.is_empty() && s.error.is_some(), "{s:?}");
        assert!(!mark.exists(), "the repository's clean filter ran");
    }

    /// A config git can't parse makes the filter lookup fail: then git
    /// status must not run (it would run with nothing blanked).
    #[tokio::test]
    async fn an_unreadable_config_turns_markers_off() {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).no_window().output();
        if ok.map(|o| !o.status.success()).unwrap_or(true) {
            return; // No git on this machine.
        }
        let config = dir.path().join(".git").join("config");
        let mut text = std::fs::read_to_string(&config).unwrap();
        text.push_str("\n[filter \"evil\"\n\tclean = broken\n");
        std::fs::write(&config, text).unwrap();
        // git refuses the repository at the first step, or the filter lookup
        // fails: either way no status runs and there are no markers.
        let s = git_status(&dir.path().to_string_lossy()).await;
        assert!(s.entries.is_empty(), "{s:?}");
        assert!(!s.in_repo || s.error.is_some(), "{s:?}");
    }

    #[tokio::test]
    async fn a_folder_outside_any_repository_has_no_markers() {
        let dir = tempfile::tempdir().unwrap();
        let s = git_status(&dir.path().to_string_lossy()).await;
        // A temp dir can sit inside a repository on some machines; then it has
        // markers, which is also right. What must hold: no error either way.
        assert!(s.error.is_none(), "{s:?}");
    }
}
