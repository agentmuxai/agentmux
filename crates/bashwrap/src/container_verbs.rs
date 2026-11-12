// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `gh` commands a container agent cannot run with its own token, and what it does instead.
//!
//! A container's GitHub token reads contents and writes pull requests, issues and the rest; it cannot write contents.
//! Two kinds of `gh` command need that:
//!
//! - `gh pr merge` merges a pull request. srv does it for the container (`POST /api/v1/github/pr-merge`) with the
//!   host's `gh-agent`, from a repository, a number and three options. Anything else a merge takes is refused here,
//!   with what to do.
//! - A raw `gh api` write that changes a ref, a tag, a release or a file (`DELETE .../git/refs/heads/x`, `PUT
//!   .../contents/f`, `POST .../releases`) would only fail at GitHub with "Resource not accessible by integration". It
//!   is refused here instead, naming the command that does the job: `git push`, `gh pr merge --delete-branch`.
//!
//! `github-container -- <gh args>` is what the `gh` function calls for a container's command. It exits 0 when it did the
//! job, 1 when it refused or failed (the reason on stderr), and 3 when the command is not one of these and `gh` should
//! run as usual.

use std::time::Duration;

use serde_json::{json, Value};

use crate::github::{default_repo_source, owner_repo_of_path, repo_of_default};
use crate::mps_client::WpsClient;

/// srv runs the merge on the host and may wait on GitHub and a merge queue.
const MERGE_TIMEOUT: Duration = Duration::from_secs(100);

/// Exit status meaning "not for this command: run `gh` as usual".
pub const NOT_HANDLED: i32 = 3;

/// A merge as the container asked for it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merge {
    pub repo: Option<(String, String)>,
    pub number: u64,
    pub method: Option<&'static str>,
    pub auto: bool,
    pub delete_branch: bool,
}

/// What `gh pr merge <args>` means: a merge srv can do, `Help` for `--help`, or why it cannot be done here.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Merge(Merge),
    Help,
}

fn unsupported(flag: &str) -> String {
    format!(
        "srv merges for a container with a fixed set of options (--squash, --merge, --rebase, --auto, --delete-branch, -R); {flag} is not one of them. \
         Merge the pull request without it, or ask your leader"
    )
}

const BY_NUMBER: &str = "name the pull request by its number: gh pr merge <number> (a branch name or the current branch cannot be merged from a container)";

/// `gh pr merge`'s arguments (`args` start with `pr merge`).
pub fn parse_merge(args: &[String]) -> Result<Parsed, String> {
    let mut m = Merge::default();
    let mut positional: Option<&str> = None;
    let mut it = args.iter().skip(2);
    while let Some(a) = it.next() {
        let a = a.as_str();
        let set_method = |m: &mut Merge, name: &'static str| match m.method {
            Some(prev) if prev != name => Err("choose one of --squash, --merge, --rebase".to_string()),
            _ => {
                m.method = Some(name);
                Ok(())
            }
        };
        match a {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--squash" | "-s" => set_method(&mut m, "squash")?,
            "--merge" | "-m" => set_method(&mut m, "merge")?,
            "--rebase" | "-r" => set_method(&mut m, "rebase")?,
            "--auto" => m.auto = true,
            "--delete-branch" | "-d" => m.delete_branch = true,
            "--admin" => {
                return Err("--admin is not available to a container agent: the repository's own rules (a review, the checks, the merge queue) decide".to_string());
            }
            "-R" | "--repo" => m.repo = Some(repo_of_flag(it.next().map(String::as_str))?),
            _ if a.starts_with("--repo=") => m.repo = Some(repo_of_flag(Some(&a["--repo=".len()..]))?),
            _ if a.starts_with('-') => return Err(unsupported(a)),
            _ => {
                if positional.replace(a).is_some() {
                    return Err("gh pr merge from a container takes one pull request".to_string());
                }
            }
        }
    }
    let target = positional.ok_or_else(|| BY_NUMBER.to_string())?;
    if let Some(rest) = target.strip_prefix("https://github.com/") {
        // https://github.com/OWNER/REPO/pull/N
        let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
        match parts.as_slice() {
            [owner, repo, "pull", n] if n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() => {
                m.number = n.parse().map_err(|_| BY_NUMBER.to_string())?;
                m.repo = m.repo.or_else(|| owner_repo_of_path(&format!("{owner}/{repo}")));
            }
            _ => return Err(BY_NUMBER.to_string()),
        }
    } else if !target.is_empty() && target.bytes().all(|b| b.is_ascii_digit()) {
        m.number = target.parse().map_err(|_| BY_NUMBER.to_string())?;
    } else {
        return Err(BY_NUMBER.to_string());
    }
    Ok(Parsed::Merge(m))
}

fn repo_of_flag(value: Option<&str>) -> Result<(String, String), String> {
    let spec = value.ok_or_else(|| "-R needs OWNER/REPO".to_string())?;
    let spec = spec.strip_prefix("github.com/").unwrap_or(spec);
    owner_repo_of_path(spec).ok_or_else(|| format!("{spec} is not OWNER/REPO"))
}

/// The refusal for a raw `gh api` write a container's token cannot make, with the command that does the job; `None` for
/// every other `gh` command. `args` are `gh`'s own, starting with `api`.
pub fn refused_command(args: &[String]) -> Option<String> {
    match args.first().map(String::as_str)? {
        "api" => refused_api(args),
        "release" => match args.get(1).map(String::as_str)? {
            "create" | "edit" | "delete" | "delete-asset" | "upload" => Some(RELEASE.to_string()),
            _ => None,
        },
        _ => None,
    }
}

const RELEASE: &str = "a container agent cannot create or change a release: the API call needs a credential that writes contents. \
Push the tag with git (git push origin <tag>, which is checked first), or ask your leader for the release workflow";

fn refused_api(args: &[String]) -> Option<String> {
    let mut method: Option<String> = None;
    let mut fields = false;
    let mut endpoint: Option<&str> = None;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-X" | "--method" => method = it.next().map(|v| v.to_ascii_uppercase()),
            a if a.starts_with("-X") && a.len() > 2 => method = Some(a[2..].to_ascii_uppercase()),
            "-H" | "--header" | "-q" | "--jq" | "-t" | "--template" | "--hostname" | "--cache" | "-p" | "--preview" => {
                it.next();
            }
            "-f" | "--raw-field" | "-F" | "--field" | "--input" => {
                fields = true;
                it.next();
            }
            a if a.starts_with("--method=") => method = Some(a["--method=".len()..].to_ascii_uppercase()),
            a if a.starts_with("--input=") || a.starts_with("--field=") || a.starts_with("--raw-field=") => fields = true,
            a if a.starts_with('-') => {}
            a => {
                endpoint.get_or_insert(a);
            }
        }
    }
    let method = method.unwrap_or_else(|| if fields { "POST".to_string() } else { "GET".to_string() });
    if method == "GET" || method == "HEAD" {
        return None;
    }
    let path = endpoint?.trim_start_matches('/');
    let rest = path.strip_prefix("repos/")?;
    let mut parts = rest.splitn(3, '/');
    let (_owner, _repo, tail) = (parts.next()?, parts.next()?, parts.next()?);
    let in_group = |prefix: &str| tail == prefix || tail.starts_with(&format!("{prefix}/"));
    if in_group("git/refs") && method == "DELETE" {
        return Some(
            "a container agent cannot delete a branch or tag through the API: the call needs a credential that writes contents. \
             Use gh pr merge --delete-branch, or git push origin --delete <branch>"
                .to_string(),
        );
    }
    if in_group("git/refs") || in_group("git/tags") || in_group("git/commits") || in_group("git/blobs") || in_group("git/trees") {
        return Some(
            "a container agent cannot create or move refs, tags or commits through the API: the call needs a credential that writes contents. \
             Use git (git commit, git push, git push origin <tag>); a push is checked before it publishes"
                .to_string(),
        );
    }
    if tail == "merges" {
        return Some("a container agent cannot merge branches through the API: use git merge and git push, or gh pr merge for a pull request".to_string());
    }
    if tail.starts_with("pulls/") && tail.ends_with("/merge") && method == "PUT" {
        return Some("use gh pr merge <number> (srv does the merge for a container); the API merge call needs a credential that writes contents".to_string());
    }
    if tail.starts_with("contents/") || in_group("contents") {
        return Some(
            "a container agent cannot change a file through the contents API: the call needs a credential that writes contents. \
             Change the file in the clone and push with git (git push), which is checked before it publishes"
                .to_string(),
        );
    }
    if in_group("releases") {
        return Some(RELEASE.to_string());
    }
    None
}

/// Whether this container's machine narrows its GitHub tokens: it has a gh-agent (`AGENTMUX_PUBLISH_GUARD` is `1`, or
/// `older-gh-agent` where it is installed but too old to narrow, which srv then says out loud). With `0`, no value or any
/// other value there is no gh-agent on the machine, the container's token comes from the person's own GitHub connection,
/// and its `gh` commands run as they always did.
pub fn narrowing_machine(marker: Option<&str>) -> bool {
    matches!(marker, Some("1" | "older-gh-agent"))
}

/// `github-container -- <gh args>`.
pub fn run_github_container(gh_args: &[String]) -> i32 {
    if !narrowing_machine(std::env::var("AGENTMUX_PUBLISH_GUARD").ok().as_deref()) {
        return NOT_HANDLED;
    }
    if let Some(why) = refused_command(gh_args) {
        eprintln!("agentmux: not run: {why}");
        return 1;
    }
    if gh_args.first().map(String::as_str) != Some("pr") || gh_args.get(1).map(String::as_str) != Some("merge") {
        return NOT_HANDLED;
    }
    let merge = match parse_merge(gh_args) {
        Ok(Parsed::Merge(m)) => m,
        Ok(Parsed::Help) => return NOT_HANDLED,
        Err(why) => {
            eprintln!("agentmux: gh pr merge not run: {why}");
            return 1;
        }
    };
    let Some((owner, repo)) = merge.repo.clone().or_else(|| std::env::var("GH_REPO").ok().and_then(|r| repo_of_flag(Some(&r)).ok())).or_else(|| default_repo_source(None).and_then(|s| repo_of_default(&s))) else {
        eprintln!("agentmux: gh pr merge not run: no repository: pass -R OWNER/REPO, or run it in a clone of the repository");
        return 1;
    };
    match ask_merge(&owner, &repo, &merge) {
        Ok(said) => {
            if !said.is_empty() {
                println!("{said}");
            }
            0
        }
        Err(why) => {
            eprintln!("agentmux: gh pr merge #{} on {owner}/{repo} was not done: {why}", merge.number);
            1
        }
    }
}

/// The request srv gets: the repository, the number and the three options, nothing else.
pub fn merge_body(owner: &str, repo: &str, m: &Merge) -> Value {
    let mut body = json!({ "owner": owner, "repo": repo, "number": m.number, "auto": m.auto, "delete_branch": m.delete_branch });
    if let Some(method) = m.method {
        body["method"] = json!(method);
    }
    body
}

fn ask_merge(owner: &str, repo: &str, m: &Merge) -> Result<String, String> {
    let client = WpsClient::from_env().ok_or_else(|| "AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY aren't set: run this from an agent's own shell".to_string())?;
    let agent_token = std::env::var("AGENTMUX_AGENT_TOKEN").ok();
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("couldn't start: {e}"))?;
    match rt.block_on(client.post_json_status("/api/v1/github/pr-merge", agent_token.as_deref(), &merge_body(owner, repo, m), MERGE_TIMEOUT)) {
        Ok((status, answer)) if (200..300).contains(&status) => Ok(answer.get("message").and_then(Value::as_str).unwrap_or_default().to_string()),
        Ok((status, answer)) => Err(answer.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("AgentMux refused ({status})"))),
        Err(e) => Err(format!("couldn't reach AgentMux: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn merge(a: &[&str]) -> Merge {
        match parse_merge(&args(a)) {
            Ok(Parsed::Merge(m)) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_merge_by_number_with_its_few_options() {
        let m = merge(&["pr", "merge", "42", "--squash", "--auto", "-d", "-R", "agentmuxai/agentmux"]);
        assert_eq!((m.number, m.method, m.auto, m.delete_branch), (42, Some("squash"), true, true));
        assert_eq!(m.repo, Some(("agentmuxai".into(), "agentmux".into())));
        let m = merge(&["pr", "merge", "https://github.com/octo/hello/pull/7"]);
        assert_eq!((m.number, m.repo, m.method), (7, Some(("octo".into(), "hello".into())), None));
        assert_eq!(merge(&["pr", "merge", "9", "--repo=octo/hello", "--rebase"]).method, Some("rebase"));
    }

    #[test]
    fn what_srv_does_not_do_is_refused_and_says_what_to_do() {
        let no = |a: &[&str]| parse_merge(&args(a)).unwrap_err();
        assert!(no(&["pr", "merge", "42", "--admin"]).contains("--admin is not available"));
        assert!(no(&["pr", "merge", "42", "--body", "x"]).contains("is not one of them"));
        assert!(no(&["pr", "merge", "42", "--match-head-commit", "abc"]).contains("--match-head-commit"));
        assert!(no(&["pr", "merge", "my-branch"]).contains("by its number"));
        assert!(no(&["pr", "merge"]).contains("by its number"));
        assert!(no(&["pr", "merge", "1", "2"]).contains("one pull request"));
        assert!(no(&["pr", "merge", "1", "--squash", "--rebase"]).contains("choose one"));
        assert!(no(&["pr", "merge", "https://github.com/o/r/issues/3"]).contains("by its number"));
        assert_eq!(parse_merge(&args(&["pr", "merge", "--help"])), Ok(Parsed::Help));
    }

    #[test]
    fn the_request_to_srv_carries_nothing_but_the_merge() {
        let m = merge(&["pr", "merge", "42", "--squash", "-d"]);
        let body = merge_body("o", "r", &m);
        assert_eq!(body, json!({ "owner": "o", "repo": "r", "number": 42, "auto": false, "delete_branch": true, "method": "squash" }));
        assert!(merge_body("o", "r", &merge(&["pr", "merge", "5"])).get("method").is_none(), "no method leaves the choice to the repository");
    }

    #[test]
    fn a_raw_api_write_that_needs_contents_write_names_the_command_that_does_the_job() {
        let said = |a: &[&str]| refused_command(&args(a));
        let del = said(&["api", "-X", "DELETE", "repos/o/r/git/refs/heads/x"]).unwrap();
        assert!(del.contains("gh pr merge --delete-branch") && del.contains("git push origin --delete"), "{del}");
        assert!(said(&["api", "--method", "DELETE", "/repos/o/r/git/refs/tags/v1"]).is_some());
        assert!(said(&["api", "repos/o/r/git/refs", "-f", "ref=refs/heads/x", "-f", "sha=abc"]).unwrap().contains("git push"));
        assert!(said(&["api", "-X", "PATCH", "repos/o/r/git/refs/heads/x", "-f", "sha=abc"]).is_some());
        assert!(said(&["api", "-X", "POST", "repos/o/r/git/tags"]).is_some());
        assert!(said(&["api", "-X", "POST", "repos/o/r/merges", "-f", "base=a", "-f", "head=b"]).unwrap().contains("git merge"));
        assert!(said(&["api", "-X", "PUT", "repos/o/r/pulls/3/merge"]).unwrap().contains("gh pr merge <number>"));
        assert!(said(&["api", "-X", "PUT", "repos/o/r/contents/docs/a.md", "-f", "message=m"]).unwrap().contains("git push"));
        assert!(said(&["api", "-X", "POST", "repos/o/r/releases", "-f", "tag_name=v1"]).unwrap().contains("release workflow"));
        assert!(said(&["release", "create", "v1"]).unwrap().contains("release workflow"));
        assert!(said(&["release", "upload", "v1", "a.zip"]).is_some());
    }

    #[test]
    fn only_a_machine_with_gh_agent_gets_these_steps() {
        assert!(narrowing_machine(Some("1")));
        assert!(narrowing_machine(Some("older-gh-agent")));
        for off in [None, Some("0"), Some(""), Some("true"), Some("yes")] {
            assert!(!narrowing_machine(off), "{off:?}");
        }
    }

    #[test]
    fn without_the_marker_every_command_runs_as_before() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for value in [None, Some("0")] {
            match value {
                Some(v) => std::env::set_var("AGENTMUX_PUBLISH_GUARD", v),
                None => std::env::remove_var("AGENTMUX_PUBLISH_GUARD"),
            }
            for cmd in [&["pr", "merge", "7", "--admin"][..], &["release", "create", "v1"], &["api", "-X", "DELETE", "repos/o/r/git/refs/heads/x"]] {
                assert_eq!(run_github_container(&args(cmd)), NOT_HANDLED, "{value:?} {cmd:?}");
            }
        }
        std::env::set_var("AGENTMUX_PUBLISH_GUARD", "1");
        assert_eq!(run_github_container(&args(&["release", "create", "v1"])), 1, "on a machine with gh-agent it is refused");
        assert_eq!(run_github_container(&args(&["issue", "list"])), NOT_HANDLED);
        std::env::remove_var("AGENTMUX_PUBLISH_GUARD");
    }

    #[test]
    fn a_method_written_attached_to_the_flag_is_read() {
        let a = args(&["api", "-XDELETE", "repos/o/r/git/refs/heads/x"]);
        assert!(refused_command(&a).is_some());
    }

    #[test]
    fn reads_and_the_writes_a_container_can_make_are_left_alone() {
        let said = |a: &[&str]| refused_command(&args(a));
        for ok in [
            &["api", "repos/o/r/git/refs/heads/x"][..],
            &["api", "repos/o/r/contents/README.md"],
            &["api", "repos/o/r/releases/latest"],
            &["api", "-X", "GET", "repos/o/r/git/refs"],
            &["api", "-X", "POST", "repos/o/r/issues/3/comments", "-f", "body=hi"],
            &["api", "-X", "PATCH", "repos/o/r/pulls/3", "-f", "title=t"],
            &["api", "-X", "PUT", "repos/o/r/pulls/3/update-branch"],
            &["api", "graphql", "-f", "query=query{viewer{login}}"],
            &["release", "list"],
            &["release", "view", "v1"],
            &["pr", "create", "--fill"],
            &["issue", "close", "3"],
        ] {
            assert_eq!(said(ok), None, "{ok:?}");
        }
    }

    #[test]
    fn a_header_value_or_a_field_name_is_not_taken_for_the_endpoint() {
        let a = args(&["api", "-H", "Accept: application/vnd.github+json", "-X", "DELETE", "repos/o/r/git/refs/heads/x"]);
        assert!(refused_command(&a).is_some());
        let b = args(&["api", "--jq", ".sha", "repos/o/r/git/refs/heads/x"]);
        assert_eq!(refused_command(&b), None);
    }
}
