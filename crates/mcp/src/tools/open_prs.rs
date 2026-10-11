// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Open PRs in `WhoIsWorkingOn` (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md
//! §3.5). They come from GitHub, so they cover agents on every computer. They
//! are read through this agent's own `gh-agent` (srv holds no GitHub
//! credential), credited to the agent named by the `<Agent>@<host>: ` title
//! prefix CI requires, and cached for 2 minutes per repository. A PR without
//! that prefix (a person's or a bot's, which the prefix rule exempts) is
//! credited to its GitHub author, with no host.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const CACHE_TTL: Duration = Duration::from_secs(120);
/// `gh pr list --limit` is a cap on results, not a page size: high enough
/// for every open PR of any repository here.
const LIST_LIMIT: &str = "1000";
const PR_FIELDS: &str = "number,title,headRefName,files,author";
const GH_TIMEOUT: Duration = Duration::from_secs(20);
/// Files named in one match's detail; the rest are counted.
const FILES_NAMED: usize = 3;

#[cfg(windows)]
const GH_AGENT: &str = "gh-agent.cmd";
#[cfg(not(windows))]
const GH_AGENT: &str = "gh-agent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenPr {
    pub number: u64,
    pub title: String,
    pub branch: String,
    pub files: Vec<String>,
    /// From the title prefix, else the PR's GitHub author.
    pub agent: String,
    /// From the title prefix; `None` without one.
    pub host: Option<String>,
}

/// `AgentY@narko: fix(x): …` → `("AgentY", "narko")`.
pub(super) fn title_author(title: &str) -> Option<(String, String)> {
    let (prefix, _) = title.split_once(": ")?;
    let (agent, host) = prefix.split_once('@')?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (ok(agent) && ok(host)).then(|| (agent.to_string(), host.to_string()))
}

/// `gh pr list --json number,title,headRefName,files,author`'s output.
pub(super) fn parse_pr_list(text: &str) -> Result<Vec<OpenPr>, String> {
    let list: Value = serde_json::from_str(text.trim()).map_err(|e| format!("gh-agent printed something other than JSON: {e}"))?;
    let items = list.as_array().ok_or("gh-agent printed something other than a list")?;
    Ok(items.iter().filter_map(parse_pr).collect())
}

/// One PR of `gh`'s JSON.
fn parse_pr(p: &Value) -> Option<OpenPr> {
    let title = p.get("title")?.as_str()?.to_string();
    let (agent, host) = match title_author(&title) {
        Some((agent, host)) => (agent, Some(host)),
        None => {
            let login = p.pointer("/author/login").and_then(Value::as_str).unwrap_or("unknown");
            (login.to_string(), None)
        }
    };
    let files = p
        .get("files")
        .and_then(Value::as_array)
        .map(|fs| fs.iter().filter_map(|f| f.get("path")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Some(OpenPr {
        number: p.get("number")?.as_u64()?,
        branch: p.get("headRefName").and_then(Value::as_str).unwrap_or_default().to_string(),
        title,
        files,
        agent,
        host,
    })
}

/// PR `number` of `repo` read on its own (`gh pr view`), when it is open:
/// for a PR asked about by number that the list didn't hold.
pub(super) async fn open_pr(repo: &str, number: u64) -> Result<Option<OpenPr>, String> {
    let (repo, n) = (repo.to_string(), number.to_string());
    let text = tokio::task::spawn_blocking(move || {
        run_gh_agent(&["pr", "view", &n, "--repo", &repo, "--json", &format!("{PR_FIELDS},state")])
    })
    .await
    .map_err(|e| format!("gh-agent: {e}"))??;
    let pr: Value = serde_json::from_str(text.trim()).map_err(|e| format!("gh-agent printed something other than JSON: {e}"))?;
    Ok(if pr.get("state").and_then(Value::as_str) == Some("OPEN") { parse_pr(&pr) } else { None })
}

/// Repository → when its PRs were read, and them.
type Cache = HashMap<String, (Instant, Vec<OpenPr>)>;

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The repository's open PRs, from the cache when it is under 2 minutes old.
pub(super) async fn open_prs(repo: &str) -> Result<Vec<OpenPr>, String> {
    if let Some((at, prs)) = cache().get(repo) {
        if at.elapsed() < CACHE_TTL {
            return Ok(prs.clone());
        }
    }
    let repo_arg = repo.to_string();
    let text = tokio::task::spawn_blocking(move || {
        run_gh_agent(&["pr", "list", "--repo", &repo_arg, "--state", "open", "--limit", LIST_LIMIT, "--json", PR_FIELDS])
    })
        .await
        .map_err(|e| format!("gh-agent: {e}"))??;
    let prs = parse_pr_list(&text)?;
    cache().insert(repo.to_string(), (Instant::now(), prs.clone()));
    Ok(prs)
}

/// `gh-agent <args>`'s output, killed after [`GH_TIMEOUT`].
fn run_gh_agent(args: &[&str]) -> Result<String, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(GH_AGENT);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    let mut child = cmd.spawn().map_err(|e| format!("can't run gh-agent: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + GH_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("gh-agent took over {} s", GH_TIMEOUT.as_secs()));
            }
            Err(e) => return Err(format!("gh-agent: {e}")),
        }
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if !status.success() {
        // Its own status lines start with `gh-agent:`; the error is the rest.
        let why = err.lines().filter(|l| !l.starts_with("gh-agent:")).collect::<Vec<_>>().join(" ");
        return Err(format!("gh-agent {} failed: {}", args.iter().take(2).copied().collect::<Vec<_>>().join(" "), why.trim()));
    }
    Ok(out)
}

/// What the caller asked, as srv resolved it (`asked`), plus `pr`.
#[derive(Debug, Default)]
pub(super) struct Asked<'a> {
    /// Repository-relative; `""` or `None` is the whole repository.
    pub path: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub query: Option<&'a str>,
    pub pr: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrMatch {
    pub agent: String,
    pub host: Option<String>,
    pub detail: String,
}

fn under(file: &str, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    file == path || file.strip_prefix(path).is_some_and(|rest| rest.starts_with('/'))
}

fn files_phrase(files: &[&String]) -> String {
    let named: Vec<&str> = files.iter().take(FILES_NAMED).map(|f| f.as_str()).collect();
    match files.len().saturating_sub(FILES_NAMED) {
        0 => named.join(", "),
        more => format!("{} and {more} more", named.join(", ")),
    }
}

/// The open PRs that touch what was asked, each credited to its agent. The
/// caller's own PRs (same agent and host) are left out. `Err` when `pr` names
/// no open PR here.
pub(super) fn pr_matches(prs: &[OpenPr], asked: &Asked, me_agent: &str, me_host: &str) -> Result<Vec<PrMatch>, String> {
    let mine = |p: &OpenPr| {
        p.agent.eq_ignore_ascii_case(me_agent) && p.host.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(me_host))
    };
    let asked_files: Option<&[String]> = match asked.pr {
        Some(n) => Some(
            &prs.iter()
                .find(|p| p.number == n)
                .ok_or_else(|| format!("PR #{n} is not an open PR in this repository"))?
                .files,
        ),
        None => None,
    };
    let path = asked.path.filter(|p| !p.is_empty());
    let words: Vec<String> = asked.query.map(|q| q.split_whitespace().map(str::to_lowercase).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for p in prs.iter().filter(|p| !mine(p) && Some(p.number) != asked.pr) {
        let label = format!("PR #{} ({})", p.number, p.title);
        let detail = if let Some(files) = asked_files {
            let shared: Vec<&String> = p.files.iter().filter(|f| files.contains(f)).collect();
            if shared.is_empty() {
                continue;
            }
            format!("{label} also changes {}", files_phrase(&shared))
        } else if let Some(path) = path {
            let touched: Vec<&String> = p.files.iter().filter(|f| under(f, path)).collect();
            if touched.is_empty() {
                continue;
            }
            format!("{label} changes {}", files_phrase(&touched))
        } else if let Some(branch) = asked.branch {
            if p.branch != branch {
                continue;
            }
            format!("{label} is on branch {branch}")
        } else if !words.is_empty() {
            let title = p.title.to_lowercase();
            if !words.iter().all(|w| title.contains(w.as_str())) {
                continue;
            }
            label
        } else {
            format!("{label} changes {} files", p.files.len())
        };
        out.push(PrMatch { agent: p.agent.clone(), host: p.host.clone(), detail });
    }
    Ok(out)
}

/// Add `matches` to `body.agents` as `pr` matches: to the local agent of
/// that name when the PR is from this computer, else as an agent of its own
/// (`channel: "github"`).
pub(super) fn merge_into(body: &mut Value, matches: Vec<PrMatch>, me_host: &str, repo: &str) {
    let Some(agents) = body.get_mut("agents").and_then(Value::as_array_mut) else {
        return;
    };
    for m in matches {
        let entry = json!({ "kind": "pr", "detail": m.detail, "since_ms": null });
        let local = m.host.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(me_host));
        let existing = agents.iter_mut().find(|a| {
            let name = a.get("agent").and_then(Value::as_str).unwrap_or_default();
            let host = a.get("host").and_then(Value::as_str);
            name.eq_ignore_ascii_case(&m.agent)
                && match (host, m.host.as_deref()) {
                    (Some(h), Some(mh)) => h.eq_ignore_ascii_case(mh),
                    (Some(_), None) => false,
                    (None, _) => local,
                }
        });
        match existing {
            Some(a) => {
                if let Some(list) = a.get_mut("matches").and_then(Value::as_array_mut) {
                    list.push(entry);
                }
            }
            None => agents.push(json!({
                "agent": m.agent,
                "host": m.host,
                "channel": "github",
                "status": "open PR",
                "goal": null,
                "repo": repo,
                "branch": null,
                "git_checked_ms": null,
                "matches": [entry],
            })),
        }
    }
}

/// This computer's name, as the `<Agent>@<host>` prefix writes it.
pub(super) fn this_host() -> String {
    sysinfo::System::host_name().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, title: &str, branch: &str, files: &[&str]) -> OpenPr {
        let (agent, host) = title_author(title).map(|(a, h)| (a, Some(h))).unwrap_or(("someone".into(), None));
        OpenPr { number, title: title.into(), branch: branch.into(), files: files.iter().map(|f| f.to_string()).collect(), agent, host }
    }

    #[test]
    fn the_title_prefix_names_the_agent() {
        assert_eq!(title_author("AgentY@narko: fix(x): y"), Some(("AgentY".into(), "narko".into())));
        assert_eq!(title_author("Manoz@Area54: chore: z"), Some(("Manoz".into(), "Area54".into())));
        assert_eq!(title_author("chore(deps): bump serde"), None);
        assert_eq!(title_author("Bump a@b from 1 to 2"), None);
        assert_eq!(title_author("a b@c: x"), None);
    }

    #[test]
    fn reads_gh_output_crediting_a_pr_without_the_prefix_to_its_author() {
        let text = r#"[
          {"number": 12, "title": "Agent4@narko: feat: x", "headRefName": "agent4/x", "author": {"login": "app/muxagent"},
           "files": [{"path": "crates/a.rs", "additions": 1, "deletions": 0}]},
          {"number": 13, "title": "Bump serde from 1.0 to 1.1", "headRefName": "dependabot/x",
           "author": {"login": "app/dependabot"}, "files": [{"path": "Cargo.lock"}]}
        ]"#;
        let prs = parse_pr_list(text).unwrap();
        assert_eq!(prs[0], pr(12, "Agent4@narko: feat: x", "agent4/x", &["crates/a.rs"]));
        assert_eq!((prs[1].agent.as_str(), prs[1].host.as_deref()), ("app/dependabot", None));
        assert_eq!(prs[1].files, vec!["Cargo.lock".to_string()]);
        assert!(parse_pr_list("not json").is_err());
    }

    fn sample() -> Vec<OpenPr> {
        vec![
            pr(10, "AgentY@narko: mine", "agenty/x", &["crates/srv/a.rs"]),
            pr(11, "Agent4@narko: presence", "agent4/p", &["crates/srv/a.rs", "crates/srv/b.rs"]),
            pr(12, "Lark@area54: widgets catalog", "lark/w", &["frontend/w.ts"]),
            pr(13, "AgentY@area54: same name, other computer", "agenty/y", &["crates/srv/a.rs"]),
        ]
    }

    #[test]
    fn a_path_matches_prs_changing_it_or_anything_under_it() {
        let asked = Asked { path: Some("crates/srv"), ..Default::default() };
        let got = pr_matches(&sample(), &asked, "AgentY", "narko").unwrap();
        let names: Vec<_> = got.iter().map(|m| (m.agent.as_str(), m.host.as_deref())).collect();
        assert_eq!(names, vec![("Agent4", Some("narko")), ("AgentY", Some("area54"))], "my own PR is left out, my namesake isn't");
        assert_eq!(got[0].detail, "PR #11 (Agent4@narko: presence) changes crates/srv/a.rs, crates/srv/b.rs");
        let file = Asked { path: Some("crates/srv/b.rs"), ..Default::default() };
        assert_eq!(pr_matches(&sample(), &file, "AgentY", "narko").unwrap().len(), 1);
        let not_a_prefix = Asked { path: Some("crates/sr"), ..Default::default() };
        assert!(pr_matches(&sample(), &not_a_prefix, "AgentY", "narko").unwrap().is_empty());
    }

    #[test]
    fn a_pr_number_finds_other_prs_changing_the_same_files() {
        let asked = Asked { pr: Some(10), ..Default::default() };
        let got = pr_matches(&sample(), &asked, "Someone", "else").unwrap();
        assert_eq!(got.iter().map(|m| m.detail.as_str()).collect::<Vec<_>>(), vec![
            "PR #11 (Agent4@narko: presence) also changes crates/srv/a.rs",
            "PR #13 (AgentY@area54: same name, other computer) also changes crates/srv/a.rs",
        ]);
        assert!(pr_matches(&sample(), &Asked { pr: Some(99), ..Default::default() }, "x", "y").is_err());
    }

    /// A person's or a bot's PR (no prefix) can be asked about, and shows as
    /// its author's.
    #[test]
    fn a_pr_without_the_prefix_can_be_asked_about_and_is_listed() {
        let mut prs = sample();
        prs.push(pr(30, "Bump serde", "dependabot/serde", &["crates/srv/a.rs"]));
        let asked = Asked { pr: Some(30), ..Default::default() };
        let got = pr_matches(&prs, &asked, "AgentY", "narko").unwrap();
        assert_eq!(got.len(), 2, "#11 and #13 share its file; my own #10 is left out");
        let by_path = pr_matches(&prs, &Asked { path: Some("crates/srv/a.rs"), ..Default::default() }, "AgentY", "narko").unwrap();
        assert!(by_path.iter().any(|m| m.agent == "someone" && m.host.is_none()));
    }

    #[test]
    fn a_branch_a_query_or_the_whole_repository() {
        let branch = Asked { branch: Some("lark/w"), ..Default::default() };
        assert_eq!(pr_matches(&sample(), &branch, "AgentY", "narko").unwrap()[0].agent, "Lark");
        let query = Asked { query: Some("Widgets CATALOG"), ..Default::default() };
        assert_eq!(pr_matches(&sample(), &query, "AgentY", "narko").unwrap().len(), 1);
        let all = pr_matches(&sample(), &Asked { path: Some(""), ..Default::default() }, "AgentY", "narko").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].detail, "PR #12 (Lark@area54: widgets catalog) changes 1 files");
    }

    #[test]
    fn many_files_are_counted() {
        let p = pr(20, "A@h: big", "a/b", &["d/1", "d/2", "d/3", "d/4", "d/5"]);
        let got = pr_matches(&[p], &Asked { path: Some("d"), ..Default::default() }, "x", "y").unwrap();
        assert_eq!(got[0].detail, "PR #20 (A@h: big) changes d/1, d/2, d/3 and 2 more");
    }

    #[test]
    fn matches_join_a_local_agent_or_stand_alone() {
        let mut body = json!({ "agents": [
            { "agent": "Agent4", "channel": "local-main", "matches": [{ "kind": "dirty", "detail": "x" }] }
        ]});
        let matches = vec![
            PrMatch { agent: "agent4".into(), host: Some("narko".into()), detail: "PR #11".into() },
            PrMatch { agent: "Agent4".into(), host: Some("area54".into()), detail: "PR #14".into() },
            PrMatch { agent: "app/dependabot".into(), host: None, detail: "PR #15".into() },
        ];
        merge_into(&mut body, matches, "narko", "o/r");
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[2]["host"], Value::Null);
        assert_eq!(agents[0]["matches"][1], json!({ "kind": "pr", "detail": "PR #11", "since_ms": null }));
        assert_eq!(agents[1]["host"], "area54");
        assert_eq!(agents[1]["channel"], "github");
    }
}
