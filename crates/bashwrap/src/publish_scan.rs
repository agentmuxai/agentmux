// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent's push, judged by the host's publish guard.
//!
//! The `pre-push` hook (`git_hooks.rs`) collects what the push publishes with the container's own git: per ref update
//! the commits' messages, the added paths and the added lines, as the text git prints. `ask` posts it to srv
//! (`POST /api/v1/github/push-scan`), which gives it as data to the host's `gh-agent scan-push`. The host never opens
//! the container's clone; the material is all it sees.
//!
//! - A machine with no publish guard (no `gh-agent`) answers `guard: none`: the hook does nothing and says nothing.
//! - A push to a repository the guard finds private is not collected at all.
//! - On a machine that has the guard, a public repository's push whose check cannot run (srv unreachable, too slow,
//!   too large, refused) is refused, with what to do.
//!
//! The git commands are the ones gh-agent's own pre-push hook runs; the two judge the same text.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use serde::Serialize;
use serde_json::Value;

use crate::mps_client::WpsClient;

/// srv runs the guard for up to a minute (a first read of the pattern list, a visibility lookup, the scan).
const ASK_TIMEOUT: Duration = Duration::from_secs(75);
/// Past this many commits a push is refused by the guard without being read.
const MAX_PUSH_COMMITS: u64 = 500;
/// The most material one push may send; the same figure srv enforces.
const MAX_MATERIAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHANGESETS: usize = 100;
const ZERO: &str = "0000000000000000000000000000000000000000";

/// One ref update of the push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_sha: String,
}

/// The material of one ref update, as srv takes it.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Part {
    pub r#ref: String,
    pub count: u64,
    pub remote_named: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_message: Option<String>,
    pub messages: String,
    pub name_status: String,
    pub patch: String,
    pub changesets: Vec<Changeset>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Changeset {
    pub path: String,
    pub text: String,
}

/// `owner/repo`, lowercase, of a github.com remote URL (https, ssh or scp form); `None` for anything else.
pub fn repo_of_remote_url(url: &str) -> Option<String> {
    let url = url.trim();
    let after = url.split_once("github.com").map(|(before, after)| (before, after))?;
    // Whatever precedes the host must end the scheme or the user: `https://`, `git@`, `ssh://git@`, or `ssh.` is ssh.github.com.
    let before = after.0;
    if !(before.is_empty() || before.ends_with('/') || before.ends_with('@') || before.ends_with('.')) {
        return None;
    }
    let rest = after.1.trim_start_matches(|c: char| c == ':' || c.is_ascii_digit()).trim_start_matches('/');
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}").to_ascii_lowercase())
}

/// The ref updates on the hook's stdin (`<local ref> <local sha> <remote ref> <remote sha>`), deletions left out.
pub fn parse_updates(input: &str) -> Vec<RefUpdate> {
    input
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f.as_slice() {
                [local_ref, local_sha, _remote_ref, remote_sha] if !local_sha.chars().all(|c| c == '0') => {
                    Some(RefUpdate { local_ref: local_ref.to_string(), local_sha: local_sha.to_string(), remote_sha: remote_sha.to_string() })
                }
                _ => None,
            }
        })
        .collect()
}

/// The `.changesets/*.md` files named in `git log -z --name-status` output: only the guard decides which of them the
/// push added, so every path of that shape is sent (at most [`MAX_CHANGESETS`]).
pub fn changeset_paths(name_status: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in name_status.split('\0') {
        let t = token.trim_start_matches('\n');
        if let Some(name) = t.strip_prefix(".changesets/") {
            if name.ends_with(".md") && !name.contains('/') && !name.is_empty() && !out.iter().any(|p| p == t) && out.len() < MAX_CHANGESETS {
                out.push(t.to_string());
            }
        }
    }
    out
}

/// The body of an annotated tag's message from `git cat-file tag`: everything after the first blank line.
pub fn tag_body(tag: &str) -> String {
    let normalized = tag.replace("\r\n", "\n");
    normalized.splitn(2, "\n\n").nth(1).unwrap_or("").to_string()
}

/// Runs git with these arguments in the current directory: its stdout, or what failed.
pub type Git<'a> = &'a dyn Fn(&[&str]) -> Result<String, String>;

/// The material of each ref update. The commands are the ones gh-agent's own pre-push hook runs, so both judge the same text.
pub fn collect(updates: &[RefUpdate], remote: &str, git: Git) -> Result<Vec<Part>, String> {
    let named = git(&["remote"])?.lines().any(|l| l.trim() == remote);
    let mut parts = Vec::new();
    for u in updates {
        // A remote tip this clone never fetched can't be excluded; leave it out.
        let known_tip = u.remote_sha != ZERO && !u.remote_sha.is_empty() && git(&["cat-file", "-e", &format!("{}^{{commit}}", u.remote_sha)]).is_ok();
        let mut range: Vec<String> = vec![u.local_sha.clone(), "--not".to_string()];
        if named {
            range.push(format!("--remotes={remote}"));
        }
        if known_tip {
            range.push(u.remote_sha.clone());
        }
        let with = |head: &[&str]| -> Vec<String> { head.iter().map(|s| s.to_string()).chain(range.iter().cloned()).collect() };
        let run = |args: Vec<String>| -> Result<String, String> {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            git(&refs)
        };
        let count: u64 = run(with(&["rev-list", "--count"]))?.trim().parse().map_err(|_| "couldn't count the commits in this push".to_string())?;
        let mut part = Part { r#ref: u.local_ref.clone(), count, remote_named: named, ..Part::default() };
        if count <= MAX_PUSH_COMMITS {
            if u.local_ref.starts_with("refs/tags/") && git(&["cat-file", "-t", &u.local_sha])?.trim() == "tag" {
                part.tag_message = Some(tag_body(&git(&["cat-file", "tag", &u.local_sha])?));
            }
            if count > 0 {
                part.messages = run(with(&["log", "--format=%H%x00%B%x1e"]))?;
                part.name_status = run(with(&["-c", "core.quotePath=false", "log", "--reverse", "-z", "--name-status", "--find-renames", "--format=%x01%H"]))?;
                for path in changeset_paths(&part.name_status) {
                    // A changeset a later commit removed names nothing.
                    if let Ok(text) = git(&["show", &format!("{}:{path}", u.local_sha)]) {
                        part.changesets.push(Changeset { path, text });
                    }
                }
                let diff = ["--format=commit %H", "-p", "--text", "--unified=0", "--no-color", "--no-ext-diff", "--no-textconv", "--find-renames"];
                let log = |kind: &[&str]| -> Vec<String> { kind.iter().chain(diff.iter()).map(|s| s.to_string()).chain(range.iter().cloned()).collect() };
                part.patch = run(log(&["log", "--no-merges"]))? + &run(log(&["log", "--merges", "--cc"]))?;
            }
        }
        parts.push(part);
        if count > MAX_PUSH_COMMITS {
            break;
        }
    }
    Ok(parts)
}

/// git in the current directory.
fn real_git(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::piped());
    // Windows: git is a console program, and this process has no console of its own.
    cmd.no_window();
    let out = cmd.output().map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.first().copied().unwrap_or(""), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What srv said.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// This machine has no publish guard, or the repository is not one it guards: nothing to do, nothing to say.
    NotNeeded,
    /// A guard that judges. `public` is whether the repository is one it guards.
    Judged { public: bool, code: i64, lines: Vec<String> },
    /// The check could not run: what to tell the agent.
    Unavailable(String),
}

/// Read srv's answer: its status and body.
pub fn read_answer(status: u16, body: &Value) -> Answer {
    let error = || body.get("error").and_then(Value::as_str).map(str::to_string);
    match status {
        200..=299 => {
            if body.get("guard").and_then(Value::as_str) == Some("none") {
                return Answer::NotNeeded;
            }
            let Some(code) = body.get("code").and_then(Value::as_i64) else {
                return Answer::Unavailable("AgentMux gave no verdict on this push".to_string());
            };
            let lines = body.get("lines").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            // An answer that doesn't say is taken as public: the guard is not switched off by silence.
            let public = body.get("public").and_then(Value::as_bool).unwrap_or(true);
            Answer::Judged { public, code, lines }
        }
        // An AgentMux without the route has no publish guard to ask.
        404 | 405 => Answer::NotNeeded,
        413 => Answer::Unavailable(error().unwrap_or_else(|| "this push is too large to check: split it into smaller pushes".to_string())),
        429 => Answer::Unavailable(error().unwrap_or_else(|| "too many pushes are being checked: wait a minute and push again".to_string())),
        _ => Answer::Unavailable(error().unwrap_or_else(|| format!("AgentMux refused to check this push ({status}): try again in a minute"))),
    }
}

#[derive(Serialize)]
struct Request<'a> {
    repo: &'a str,
    allow_new_docs: bool,
    parts: &'a [Part],
}

/// Ask srv to judge `parts` (empty: only whether this machine guards `repo`).
fn ask(repo: &str, allow_new_docs: bool, parts: &[Part]) -> Answer {
    let Some(client) = WpsClient::from_env() else {
        return Answer::Unavailable("AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY aren't set: run this from an agent's own shell".to_string());
    };
    let agent_token = std::env::var("AGENTMUX_AGENT_TOKEN").ok();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Answer::Unavailable(format!("couldn't start: {e}")),
    };
    let body = Request { repo, allow_new_docs, parts };
    match rt.block_on(client.post_json_status("/api/v1/github/push-scan", agent_token.as_deref(), &body, ASK_TIMEOUT)) {
        Ok((status, answer)) => read_answer(status, &answer),
        Err(e) => Answer::Unavailable(format!("couldn't reach AgentMux to check this push: {e}")),
    }
}

/// What to do after a check that could not run.
fn refuse_unavailable(why: &str) -> i32 {
    eprintln!("agentmux: this push to a public repository was not checked, so it is not sent: {why}");
    eprintln!("agentmux: try the push again; if it keeps failing, ask your leader.");
    1
}

fn show(lines: &[String]) {
    for line in lines {
        eprintln!("{line}");
    }
}

/// The publish guard's part of `pre-push <remote> <url>`: 0 to go on, anything else refuses the push.
pub fn check_push(remote: &str, url: &str, input: &str) -> i32 {
    let Some(repo) = repo_of_remote_url(url) else { return 0 };
    let updates = parse_updates(input);
    if updates.is_empty() {
        return 0;
    }
    let allow = std::env::var("GH_AGENT_ALLOW_NEW_DOCS").is_ok_and(|v| v == "1");
    // First only whether this machine guards this repository, so a private one's push is not read.
    match ask(&repo, allow, &[]) {
        Answer::NotNeeded => return 0,
        Answer::Unavailable(why) => return refuse_unavailable(&why),
        Answer::Judged { public: false, .. } => return 0,
        Answer::Judged { public: true, .. } => {}
    }
    let parts = match collect(&updates, remote, &real_git) {
        Ok(p) => p,
        Err(why) => return refuse_unavailable(&why),
    };
    let size: usize = parts.iter().map(|p| p.tag_message.as_deref().map_or(0, str::len) + p.messages.len() + p.name_status.len() + p.patch.len() + p.changesets.iter().map(|c| c.path.len() + c.text.len()).sum::<usize>()).sum();
    if size > MAX_MATERIAL_BYTES {
        return refuse_unavailable("it is too large to check: split it into smaller pushes");
    }
    match ask(&repo, allow, &parts) {
        Answer::NotNeeded => 0,
        Answer::Unavailable(why) => refuse_unavailable(&why),
        Answer::Judged { code, lines, .. } => {
            show(&lines);
            i32::from(code != 0)
        }
    }
}

#[allow(dead_code)]
fn flush_stderr() {
    let _ = std::io::stderr().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[test]
    fn a_github_remote_is_owner_slash_repo_in_lowercase_and_nothing_else_is() {
        for (url, want) in [
            ("https://github.com/AgentMuxAI/agentmux.git", Some("agentmuxai/agentmux")),
            ("https://github.com/agentmuxai/agentmux", Some("agentmuxai/agentmux")),
            ("https://github.com/agentmuxai/agentmux/", Some("agentmuxai/agentmux")),
            ("git@github.com:agentmuxai/agentmux.git", Some("agentmuxai/agentmux")),
            ("ssh://git@github.com/agentmuxai/agentmux.git", Some("agentmuxai/agentmux")),
            ("ssh://git@ssh.github.com:443/agentmuxai/agentmux.git", Some("agentmuxai/agentmux")),
            ("https://x-access-token:t@github.com/o/r.git", Some("o/r")),
            ("https://notgithub.com/o/r.git", None),
            ("https://github.com.evil.example/o/r.git", None),
            ("https://gitlab.com/o/r.git", None),
            ("/some/local/path", None),
            ("https://github.com/onlyowner", None),
            ("", None),
        ] {
            assert_eq!(repo_of_remote_url(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn the_updates_are_read_and_deletions_are_left_out() {
        let sha = "a".repeat(40);
        let input = format!("refs/heads/x {sha} refs/heads/x {ZERO}\n(delete) {ZERO} refs/heads/y {sha}\n\nbroken line\n");
        let got = parse_updates(&input);
        assert_eq!(got, vec![RefUpdate { local_ref: "refs/heads/x".into(), local_sha: sha, remote_sha: ZERO.into() }]);
    }

    #[test]
    fn changesets_are_found_among_the_paths_git_lists() {
        let out = "\x01abc\0\nA\0.changesets/one.md\0\nA\0src/a.rs\0\x01def\0\nR100\0.changesets/two.md\0.changesets/three.md\0\nA\0.changesets/dir/x.md\0\nA\0.changesets/README.txt\0";
        assert_eq!(changeset_paths(out), vec![".changesets/one.md", ".changesets/two.md", ".changesets/three.md"]);
        assert!(changeset_paths("").is_empty());
    }

    #[test]
    fn a_tags_message_is_what_follows_its_header() {
        assert_eq!(tag_body("object abc\ntype commit\ntag v1\n\nthe notes\nmore"), "the notes\nmore");
        assert_eq!(tag_body("object abc\r\n\r\nnotes"), "notes");
        assert_eq!(tag_body("object abc"), "");
    }

    /// A fake git that answers by the first words of the command, and remembers what was run.
    struct FakeGit {
        answers: HashMap<&'static str, String>,
        ran: RefCell<Vec<String>>,
    }

    impl FakeGit {
        fn run(&self, args: &[&str]) -> Result<String, String> {
            let line = args.join(" ");
            self.ran.borrow_mut().push(line.clone());
            let key = ["remote", "rev-list", "cat-file -e", "cat-file -t", "cat-file tag", "show", "log --format=%H", "-c core.quotePath=false log", "log --no-merges", "log --merges"]
                .into_iter()
                .find(|k| line.starts_with(k))
                .ok_or_else(|| format!("unexpected: {line}"))?;
            self.answers.get(key).cloned().ok_or_else(|| format!("fails: {line}"))
        }
    }

    fn fake(count: &str) -> FakeGit {
        let answers = HashMap::from([
            ("remote", "origin\nupstream\n".to_string()),
            ("rev-list", format!("{count}\n")),
            ("cat-file -e", String::new()),
            ("log --format=%H", "abc\0unrelated\n\x1e".to_string()),
            ("-c core.quotePath=false log", "\x01abc\0\nA\0.changesets/c.md\0\nA\0plans/PLAN_X.md\0".to_string()),
            ("show", "the changeset".to_string()),
            ("log --no-merges", "commit abc\n+line\n".to_string()),
            ("log --merges", String::new()),
        ]);
        FakeGit { answers, ran: RefCell::new(Vec::new()) }
    }

    fn update(local_ref: &str, remote_sha: &str) -> RefUpdate {
        RefUpdate { local_ref: local_ref.into(), local_sha: "b".repeat(40), remote_sha: remote_sha.into() }
    }

    #[test]
    fn the_material_is_what_gh_agents_own_hook_would_read_and_nothing_more() {
        let git = fake("2");
        let parts = collect(&[update("refs/heads/x", &"c".repeat(40))], "origin", &|a| git.run(a)).unwrap();
        assert_eq!(parts.len(), 1);
        let p = &parts[0];
        assert_eq!((p.r#ref.as_str(), p.count, p.remote_named), ("refs/heads/x", 2, true));
        assert_eq!(p.messages, "abc\0unrelated\n\x1e");
        assert!(p.name_status.contains("plans/PLAN_X.md"));
        assert_eq!(p.patch, "commit abc\n+line\n");
        assert_eq!(p.changesets, vec![Changeset { path: ".changesets/c.md".into(), text: "the changeset".into() }]);
        assert!(p.tag_message.is_none());
        let ran = git.ran.borrow().join("\n");
        // The range: the tip, then what the named remote's refs and the remote's tip already have.
        assert!(ran.contains(&format!("rev-list --count {} --not --remotes=origin {}", "b".repeat(40), "c".repeat(40))), "{ran}");
        assert!(ran.contains("-c core.quotePath=false log --reverse -z --name-status --find-renames --format=%x01%H"), "{ran}");
        assert!(ran.contains("log --no-merges --format=commit %H -p --text --unified=0 --no-color --no-ext-diff --no-textconv --find-renames"), "{ran}");
        assert!(ran.contains("log --merges --cc --format=commit %H -p"), "{ran}");
    }

    #[test]
    fn a_bare_url_excludes_no_remote_and_a_new_branch_has_no_tip_to_exclude() {
        let git = fake("1");
        let parts = collect(&[update("refs/heads/new", ZERO)], "https://github.com/o/r.git", &|a| git.run(a)).unwrap();
        assert!(!parts[0].remote_named);
        let ran = git.ran.borrow().join("\n");
        assert!(ran.contains(&format!("rev-list --count {} --not\n", "b".repeat(40))) || ran.contains(&format!("rev-list --count {} --not", "b".repeat(40))), "{ran}");
        assert!(!ran.contains("--remotes="), "{ran}");
    }

    #[test]
    fn far_more_commits_than_a_normal_push_are_counted_and_not_read() {
        let git = fake("501");
        let parts = collect(&[update("refs/heads/x", ZERO)], "origin", &|a| git.run(a)).unwrap();
        assert_eq!(parts[0].count, 501);
        assert!(parts[0].messages.is_empty() && parts[0].patch.is_empty());
        assert!(!git.ran.borrow().iter().any(|l| l.starts_with("log")), "no log was run");
    }

    #[test]
    fn a_git_failure_is_a_failure_not_an_empty_push() {
        let mut git = fake("1");
        git.answers.remove("log --no-merges");
        assert!(collect(&[update("refs/heads/x", ZERO)], "origin", &|a| git.run(a)).is_err());
        let mut git = fake("x");
        git.answers.insert("rev-list", "not a number".to_string());
        assert!(collect(&[update("refs/heads/x", ZERO)], "origin", &|a| git.run(a)).is_err());
    }

    #[test]
    fn what_srv_says_is_read_and_a_silence_does_not_switch_the_guard_off() {
        assert_eq!(read_answer(200, &json!({ "guard": "none", "code": 0, "lines": [] })), Answer::NotNeeded);
        assert_eq!(read_answer(404, &Value::Null), Answer::NotNeeded);
        assert_eq!(
            read_answer(200, &json!({ "guard": "gh-agent", "code": 1, "lines": ["no", "way"], "public": true })),
            Answer::Judged { public: true, code: 1, lines: vec!["no".into(), "way".into()] }
        );
        assert_eq!(read_answer(200, &json!({ "guard": "gh-agent", "code": 0, "lines": [], "public": false })), Answer::Judged { public: false, code: 0, lines: vec![] });
        // no `public` field: taken as public
        assert_eq!(read_answer(200, &json!({ "guard": "gh-agent", "code": 0, "lines": [] })), Answer::Judged { public: true, code: 0, lines: vec![] });
        // no verdict at all
        assert!(matches!(read_answer(200, &json!({ "guard": "gh-agent" })), Answer::Unavailable(_)));
        for status in [400, 401, 403, 429, 500, 503] {
            assert!(matches!(read_answer(status, &json!({ "error": "why" })), Answer::Unavailable(w) if w == "why"), "{status}");
        }
        assert!(matches!(read_answer(413, &Value::Null), Answer::Unavailable(w) if w.contains("split it")));
    }

    #[test]
    fn the_request_is_the_shape_srv_takes() {
        let parts = vec![Part { r#ref: "refs/heads/x".into(), count: 1, remote_named: true, messages: "m".into(), ..Part::default() }];
        let body = serde_json::to_value(Request { repo: "o/r", allow_new_docs: true, parts: &parts }).unwrap();
        assert_eq!(body["repo"], "o/r");
        assert_eq!(body["allow_new_docs"], true);
        assert_eq!(body["parts"][0]["ref"], "refs/heads/x");
        assert!(body["parts"][0].get("tag_message").is_none());
        assert_eq!(body["parts"][0]["changesets"], json!([]));
    }
}
