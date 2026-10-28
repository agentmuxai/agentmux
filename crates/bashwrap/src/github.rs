// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! GitHub tokens for an agent's `git` and `gh`.
//!
//! - `git-credential <get|store|erase>`: git's credential helper protocol. srv puts it in the
//!   environment of an agent whose person connected GitHub through AgentMux Cloud
//!   (`credential.https://github.com.helper`), after an empty helper that clears the list, so no
//!   other helper's login (the person's, a stored token) is ever asked. For `get` it asks srv for
//!   the AgentMux GitHub App's token for the repository in the request's path; `store` and `erase`
//!   do nothing, so a token is never written down.
//! - `github-token -- <gh args>`: what the `gh` shell function this wrapper exports calls
//!   (`shim_functions.rs`). It finds the repository the command is about (`-R`, `GH_REPO`, the
//!   path of `gh api repos/o/r/…`, else the working directory's `origin`) and prints a token for
//!   it, or prints nothing and says why on stderr. The function then runs `gh` with that token, or
//!   with none: never with another credential.
//!
//! Both ask `POST /api/v1/github/token`; who is asking is decided by srv from the agent's token
//! (`AGENTMUX_AGENT_TOKEN`), and the answer is the narrowed installation token or a sentence for
//! the person. The token is never logged here.

use std::io::{Read, Write};
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use serde_json::{json, Value};

use crate::mps_client::WpsClient;

/// srv may call AgentMux Cloud twice (a revalidation, then the mint); a stuck call must not hold a
/// `git push` forever.
const ASK_TIMEOUT: Duration = Duration::from_secs(30);

/// The repository named by a git credential request's `path`: `owner/repo` or `owner/repo.git`.
pub fn owner_repo_of_path(path: &str) -> Option<(String, String)> {
    let path = path.trim().trim_matches('/');
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    // Anything after the repository (a `/info/refs`-style tail) is ignored.
    (!owner.is_empty() && !repo.is_empty()).then(|| (owner.to_string(), repo.to_string()))
}

/// `key=value` lines up to a blank line, as git writes them.
pub fn parse_request(input: &str) -> Vec<(String, String)> {
    input
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim_end_matches('\r').to_string())))
        .collect()
}

fn field<'a>(request: &'a [(String, String)], key: &str) -> Option<&'a str> {
    request.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// What git is told when srv gave a token.
pub fn credential_response(token: &str) -> String {
    format!("username=x-access-token\npassword={token}\n\n")
}

/// `owner/repo` of a GitHub remote URL (https, scp-style ssh or ssh://), for github.com only.
pub fn repo_of_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = ["https://github.com/", "http://github.com/", "ssh://git@github.com/", "git@github.com:", "git://github.com/"]
        .iter()
        .find_map(|p| url.strip_prefix(p))
        .or_else(|| url.strip_prefix("https://").and_then(|r| r.split_once('@')).and_then(|(_, r)| r.strip_prefix("github.com/")))?;
    owner_repo_of_path(rest)
}

/// The repository `gh` is about to act on, from its arguments and environment, else from the working
/// directory's `origin`. `None` when the command names no repository (e.g. `gh --version`).
pub fn repo_of_gh_command(args: &[String], env_repo: Option<&str>, origin_url: impl FnOnce() -> Option<String>) -> Option<(String, String)> {
    // -R/--repo [HOST/]OWNER/REPO
    let mut explicit: Option<String> = None;
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if a == "-R" || a == "--repo" {
            explicit = it.next().cloned();
        } else if let Some(v) = a.strip_prefix("--repo=").or_else(|| a.strip_prefix("-R=")) {
            explicit = Some(v.to_string());
        }
    }
    if let Some(r) = explicit.as_deref().filter(|r| !r.is_empty()) {
        return repo_of_spec(r);
    }
    let env_repo = env_repo.filter(|r| !r.is_empty());
    // gh api repos/OWNER/REPO/…: a literal endpoint names its repository, whatever GH_REPO says.
    if args.first().map(String::as_str) == Some("api") {
        for a in &args[1..] {
            let p = a.trim_start_matches('/');
            if let Some(rest) = p.strip_prefix("repos/") {
                // gh fills `{owner}/{repo}` from GH_REPO, else the working directory's repository.
                if rest.starts_with("{owner}/{repo}") {
                    return match env_repo {
                        Some(r) => repo_of_spec(r),
                        None => repo_of_default(&origin_url()?),
                    };
                }
                if let Some(found) = owner_repo_of_path(rest) {
                    return Some(found);
                }
            }
        }
    }
    // A pull request or issue named by its URL (`gh pr view <url>`), or a repository named on the command line.
    if let Some(found) = args.get(2).filter(|_| args[0] != "api").and_then(|a| url_repo(a)).or_else(|| positional_repo(args)) {
        return Some(found);
    }
    if let Some(r) = env_repo {
        return repo_of_spec(r);
    }
    // gh needs no repository for these.
    if matches!(args.first().map(String::as_str), None | Some("--version" | "--help" | "help" | "version" | "auth" | "config" | "completion" | "alias" | "extension" | "ext" | "gist" | "search")) {
        return None;
    }
    repo_of_default(&origin_url()?)
}

/// The repository for what [`default_repo_source`] found: a remote's URL, or an `OWNER/REPO` set with
/// `gh repo set-default`.
pub fn repo_of_default(source: &str) -> Option<(String, String)> {
    repo_of_remote(source).or_else(|| repo_of_spec(source))
}

/// What `gh` takes as the default repository, from the lines of
/// `git config --get-regexp '^remote\..*\.gh-resolved$'` (`gh repo set-default`, and a fork cloned
/// with `gh repo clone`): `Ok(OWNER/REPO)` as set, `Err(remote)` for "the repository of that remote".
pub fn resolved_default(config_lines: &str) -> Option<Result<String, String>> {
    config_lines.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(char::is_whitespace)?;
        let remote = key.strip_prefix("remote.")?.strip_suffix(".gh-resolved")?;
        match value.trim() {
            "" | "other" => None,
            "base" => Some(Err(remote.to_string())),
            repo => Some(Ok(repo.to_string())),
        }
    })
}

/// The repository of a `https://github.com/OWNER/REPO/…` argument (`gh pr view <url>`).
fn url_repo(arg: &str) -> Option<(String, String)> {
    let rest = arg.strip_prefix("https://github.com/").or_else(|| arg.strip_prefix("http://github.com/"))?;
    owner_repo_of_path(rest)
}

/// The repository `gh repo <verb> OWNER/REPO` is about, when it takes one as an argument.
fn positional_repo(args: &[String]) -> Option<(String, String)> {
    if args.first().map(String::as_str) != Some("repo") {
        return None;
    }
    if !matches!(args.get(1).map(String::as_str), Some("clone" | "view" | "fork" | "edit" | "delete" | "archive" | "rename" | "sync" | "set-default")) {
        return None;
    }
    args[2..].iter().filter(|a| !a.starts_with('-')).find_map(|a| repo_of_spec(a))
}

/// `[HOST/]OWNER/REPO`; a host other than github.com is not ours.
fn repo_of_spec(spec: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = spec.trim().split('/').collect();
    match parts.as_slice() {
        [owner, repo] => owner_repo_of_path(&format!("{owner}/{repo}")),
        [host, owner, repo] if host.eq_ignore_ascii_case("github.com") => owner_repo_of_path(&format!("{owner}/{repo}")),
        _ => None,
    }
}

/// What `POST /api/v1/github/token` granted: the token, who the App's bot is, and who the agent
/// works for (the account's own GitHub id and login).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Granted {
    pub token: String,
    pub owner_github_id: Option<u64>,
    pub owner_login: Option<String>,
    pub bot_slug: Option<String>,
    pub bot_user_id: Option<u64>,
}

/// The answer of `POST /api/v1/github/token`: a token, or what to tell the person.
pub enum Answer {
    Token(Granted),
    Refused(String),
}

/// Ask srv for a token for `owner/repo`.
pub fn ask(owner: &str, repo: &str) -> Answer {
    let Some(client) = WpsClient::from_env() else {
        return Answer::Refused("AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY aren't set: run this from an agent's own shell".to_string());
    };
    let agent_token = std::env::var("AGENTMUX_AGENT_TOKEN").ok();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Answer::Refused(format!("couldn't start: {e}")),
    };
    let body = json!({ "owner": owner, "repo": repo });
    match rt.block_on(client.post_json_status("/api/v1/github/token", agent_token.as_deref(), &body, ASK_TIMEOUT)) {
        Ok((status, answer)) if (200..300).contains(&status) => match answer.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            Some(token) => Answer::Token(Granted {
                token: token.to_string(),
                owner_github_id: answer.get("ownerGithubId").and_then(Value::as_u64),
                owner_login: answer.get("ownerLogin").and_then(Value::as_str).map(str::to_string),
                bot_slug: answer.get("botSlug").and_then(Value::as_str).map(str::to_string),
                bot_user_id: answer.get("botUserId").and_then(Value::as_u64),
            }),
            None => Answer::Refused("AgentMux gave no token".to_string()),
        },
        Ok((status, answer)) => Answer::Refused(
            answer.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("AgentMux refused ({status})")),
        ),
        Err(e) => Answer::Refused(format!("couldn't reach AgentMux: {e}")),
    }
}

/// `git-credential <op>`: git's helper protocol. Exit code 0 with the credential on stdout, or 1.
pub fn run_git_credential(op: &str) -> i32 {
    // Never store: a token lasts an hour and is minted per request.
    if op != "get" {
        return 0;
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let request = parse_request(&input);
    if field(&request, "protocol") != Some("https") || !field(&request, "host").is_some_and(|h| h.eq_ignore_ascii_case("github.com")) {
        // Not GitHub: nothing to give, and nothing to say.
        return 1;
    }
    let Some((owner, repo)) = field(&request, "path").and_then(owner_repo_of_path) else {
        eprintln!("agentmux: git asked for GitHub credentials without a repository, so there is no token to give (credential.useHttpPath is off?)");
        return 1;
    };
    match ask(&owner, &repo) {
        Answer::Token(granted) => {
            let mut out = std::io::stdout();
            let _ = out.write_all(credential_response(&granted.token).as_bytes());
            let _ = out.flush();
            0
        }
        Answer::Refused(why) => {
            eprintln!("agentmux: no GitHub token for {owner}/{repo}: {why}");
            1
        }
    }
}

/// The repository `gh` itself would act on, as a remote's URL or an `OWNER/REPO`: the one it was
/// told (`gh repo set-default`, a cloned fork's parent), else `origin`'s. `cwd` is where `git -C` points.
pub fn default_repo_source(cwd: Option<&str>) -> Option<String> {
    let git_config = |args: &[&str]| {
        let mut cmd = std::process::Command::new("git");
        if let Some(dir) = cwd {
            cmd.arg("-C").arg(dir);
        }
        cmd.arg("config").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // Windows: git is a console program, and this process has no console of its own.
        cmd.no_window();
        let out = cmd.output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    match resolved_default(&git_config(&["--get-regexp", r"^remote\..*\.gh-resolved$"]).unwrap_or_default()) {
        Some(Ok(spec)) => Some(spec),
        Some(Err(remote)) => git_config(&["--get", &format!("remote.{remote}.url")]),
        None => git_config(&["--get", "remote.origin.url"]),
    }
}

/// The repository a `gh` command is about, for [`run_github_token`] and [`run_github_owner_tag`].
fn repo_of_command_here(gh_args: &[String]) -> Option<(String, String)> {
    let env_repo = std::env::var("GH_REPO").ok();
    repo_of_gh_command(gh_args, env_repo.as_deref(), || default_repo_source(None))
}

/// What marks a PR as the work of a person's agent: the id of the GitHub account it works for.
pub fn owner_tag(owner_github_id: u64) -> String {
    format!("<!-- agentmux:owner_github_id={owner_github_id} -->")
}

fn with_tag(body: &str, tag: &str) -> String {
    if body.contains("agentmux:owner_github_id=") {
        body.to_string()
    } else if body.trim().is_empty() {
        format!("{tag}\n")
    } else {
        format!("{}\n\n{tag}\n", body.trim_end())
    }
}

/// `gh pr create` / `gh pr edit` arguments with `tag` added to the PR's body: `--body`/`-b` text,
/// or a `--body-file`/`-F` file (copied with the tag, by `file_with_tag`; stdin can't be). Arguments
/// that give no body (`--fill`, a prompt) are left alone.
pub fn rewrite_pr_body(args: &[String], tag: &str, file_with_tag: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = args.get(i + 1);
        match (a, value) {
            ("--body" | "-b", Some(v)) => {
                out.extend([a.to_string(), with_tag(v, tag)]);
                i += 2;
            }
            ("--body-file" | "-F", Some(v)) => {
                let file = if v == "-" { v.clone() } else { file_with_tag(v).unwrap_or_else(|| v.clone()) };
                out.extend([a.to_string(), file]);
                i += 2;
            }
            _ => {
                if let Some(v) = a.strip_prefix("--body=") {
                    out.push(format!("--body={}", with_tag(v, tag)));
                } else if let Some(v) = a.strip_prefix("--body-file=") {
                    out.push(format!("--body-file={}", file_with_tag(v).unwrap_or_else(|| v.to_string())));
                } else {
                    out.push(a.to_string());
                }
                i += 1;
            }
        }
    }
    out
}

/// A copy of the body file at `path` with `tag` added, in the temp directory.
fn body_file_with_tag(path: &str, tag: &str) -> Option<String> {
    let body = std::fs::read_to_string(path).ok()?;
    let text = with_tag(&body, tag);
    let name = format!("agentmux-pr-body-{}-{:x}.md", std::process::id(), text.len());
    let copy = std::env::temp_dir().join(name);
    std::fs::write(&copy, text).ok()?;
    Some(copy.to_string_lossy().into_owned())
}

/// `github-owner-tag -- <gh args>`: the arguments of a `gh pr create` or `gh pr edit` with the owner
/// tag in its body, each followed by a NUL byte, for the `gh` function to run `gh` with. The
/// arguments unchanged when there is no owner to name or no body to put it in.
pub fn run_github_owner_tag(gh_args: &[String]) -> i32 {
    let mut args = gh_args.to_vec();
    if let Some((owner, repo)) = repo_of_command_here(gh_args) {
        if let Answer::Token(Granted { owner_github_id: Some(id), .. }) = ask(&owner, &repo) {
            let tag = owner_tag(id);
            args = rewrite_pr_body(gh_args, &tag, |path| body_file_with_tag(path, &tag));
        }
    }
    let mut out = std::io::stdout();
    for a in &args {
        let _ = out.write_all(a.as_bytes());
        let _ = out.write_all(&[0]);
    }
    let _ = out.flush();
    0
}

/// `github-token -- <gh args>`: print a token for the repository the `gh` command is about; with
/// none, say why on stderr (when there is a repository) and print nothing.
pub fn run_github_token(gh_args: &[String]) -> i32 {
    let Some((owner, repo)) = repo_of_command_here(gh_args) else {
        return 0;
    };
    match ask(&owner, &repo) {
        Answer::Token(granted) => {
            println!("{}", granted.token);
            0
        }
        Answer::Refused(why) => {
            eprintln!("agentmux: no GitHub token for {owner}/{repo}: {why}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn repo(o: &str, r: &str) -> Option<(String, String)> {
        Some((o.to_string(), r.to_string()))
    }

    /// srv, as a one-shot stub: answers one request with `status` and `body`, and returns what it was sent.
    fn stub_srv(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap();
                data.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&data).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&data).to_string()
        });
        (url, handle)
    }

    #[test]
    fn a_token_comes_from_srv_for_the_repository_as_the_calling_agent() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, srv) = stub_srv(200, r#"{"token":"ghs_FROM_SRV","expiresAt":1,"botSlug":"muxagent","botUserId":7,"ownerGithubId":42,"ownerLogin":"octo"}"#);
        std::env::set_var("AGENTMUX_LOCAL_URL", &url);
        std::env::set_var("AGENTMUX_AUTH_KEY", "key");
        std::env::set_var("AGENTMUX_AGENT_TOKEN", "agent-token-1");
        let answer = ask("octo", "hello");
        let seen = srv.join().unwrap();
        assert!(matches!(&answer, Answer::Token(t) if t.token == "ghs_FROM_SRV" && t.owner_github_id == Some(42) && t.bot_slug.as_deref() == Some("muxagent")));
        assert!(seen.starts_with("POST /api/v1/github/token "), "{seen}");
        let lower = seen.to_ascii_lowercase();
        assert!(lower.contains("x-agent-token: agent-token-1"), "who is asking is the agent's own token: {seen}");
        assert!(seen.contains(r#""owner":"octo""#) && seen.contains(r#""repo":"hello""#), "{seen}");
        assert!(!seen.contains("agent_id"), "no name in the request: srv decides who is asking");
    }

    #[test]
    fn a_refusal_is_srvs_own_sentence_never_a_token() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (url, srv) = stub_srv(403, r#"{"error":"the AgentMux GitHub App isn't installed on octo"}"#);
        std::env::set_var("AGENTMUX_LOCAL_URL", &url);
        std::env::set_var("AGENTMUX_AUTH_KEY", "key");
        std::env::remove_var("AGENTMUX_AGENT_TOKEN");
        let answer = ask("octo", "hello");
        srv.join().unwrap();
        assert!(matches!(&answer, Answer::Refused(why) if why == "the AgentMux GitHub App isn't installed on octo"), "refused with srv's words");
    }

    #[test]
    fn the_owner_tag_goes_into_the_body_however_it_is_given() {
        let tag = owner_tag(42);
        assert_eq!(tag, "<!-- agentmux:owner_github_id=42 -->");
        let copy = |p: &str| Some(format!("{p}.tagged"));
        let got = rewrite_pr_body(&args(&["pr", "create", "--title", "t", "--body", "Text\n"]), &tag, copy);
        assert_eq!(got[5], format!("Text\n\n{tag}\n"));
        let got = rewrite_pr_body(&args(&["pr", "edit", "-b", "x", "--body=y", "--body-file", "f.md", "-F", "-", "--body-file=g.md"]), &tag, copy);
        assert_eq!(got[2], "-b");
        assert!(got[3].ends_with(&format!("{tag}
")));
        assert_eq!(got[4], format!("--body=y

{tag}
"));
        assert_eq!(&got[5..7], &["--body-file".to_string(), "f.md.tagged".to_string()]);
        assert_eq!(&got[7..9], &["-F".to_string(), "-".to_string()], "stdin can't be copied");
        assert_eq!(got[9], "--body-file=g.md.tagged");
        // Already there: not twice. No body: nothing to add it to.
        let again = rewrite_pr_body(&args(&["pr", "create", "-b", &format!("x\n{tag}")]), &tag, copy);
        assert_eq!(again[3], format!("x\n{tag}"));
        assert_eq!(rewrite_pr_body(&args(&["pr", "create", "--fill"]), &tag, copy), args(&["pr", "create", "--fill"]));
        assert_eq!(rewrite_pr_body(&args(&["pr", "create", "-b", ""]), &tag, copy)[3], format!("{tag}\n"));
    }

    #[test]
    fn gh_resolved_names_the_default_repository() {
        assert_eq!(resolved_default("remote.upstream.gh-resolved base\nremote.origin.gh-resolved other"), Some(Err("upstream".to_string())));
        assert_eq!(resolved_default("remote.origin.gh-resolved octo/hello"), Some(Ok("octo/hello".to_string())));
        assert_eq!(resolved_default("remote.origin.gh-resolved other"), None);
        assert_eq!(resolved_default(""), None);
    }

    #[test]
    fn git_asks_with_key_value_lines_and_a_path() {
        let input = "protocol=https\nhost=github.com\npath=octo/hello.git\n\nignored=after-blank\n";
        let r = parse_request(input);
        assert_eq!(field(&r, "host"), Some("github.com"));
        assert_eq!(field(&r, "path").and_then(owner_repo_of_path), repo("octo", "hello"));
        assert_eq!(field(&r, "ignored"), None);
        assert_eq!(parse_request("protocol=https\r\nhost=github.com\r\n\r\n").len(), 2);
    }

    #[test]
    fn a_path_names_the_repository_and_nothing_else_does() {
        assert_eq!(owner_repo_of_path("octo/hello"), repo("octo", "hello"));
        assert_eq!(owner_repo_of_path("/octo/hello.git/"), repo("octo", "hello"));
        assert_eq!(owner_repo_of_path("octo/hello/info/refs"), repo("octo", "hello"));
        assert_eq!(owner_repo_of_path("octo"), None);
        assert_eq!(owner_repo_of_path(""), None);
        assert_eq!(owner_repo_of_path("/hello"), None);
    }

    #[test]
    fn the_credential_is_a_username_and_the_token_and_nothing_stored() {
        assert_eq!(credential_response("ghs_X"), "username=x-access-token\npassword=ghs_X\n\n");
        // store and erase are never an error and never keep anything.
        assert_eq!(run_git_credential("store"), 0);
        assert_eq!(run_git_credential("erase"), 0);
    }

    #[test]
    fn github_remotes_in_every_spelling_name_the_repository() {
        for url in [
            "https://github.com/octo/hello.git",
            "https://github.com/octo/hello",
            "https://user@github.com/octo/hello.git",
            "git@github.com:octo/hello.git",
            "ssh://git@github.com/octo/hello.git",
            "git://github.com/octo/hello",
        ] {
            assert_eq!(repo_of_remote(url), repo("octo", "hello"), "{url}");
        }
        assert_eq!(repo_of_remote("https://gitlab.com/octo/hello.git"), None);
        assert_eq!(repo_of_remote("https://github.example.com/octo/hello.git"), None);
        assert_eq!(repo_of_remote("/home/me/repo"), None);
    }

    #[test]
    fn gh_is_about_the_repository_it_names_else_the_working_directorys() {
        let origin = || Some("https://github.com/home/checkout.git".to_string());
        assert_eq!(repo_of_gh_command(&args(&["pr", "create"]), None, origin), repo("home", "checkout"));
        assert_eq!(repo_of_gh_command(&args(&["pr", "list", "-R", "octo/hello"]), None, origin), repo("octo", "hello"));
        assert_eq!(repo_of_gh_command(&args(&["pr", "list", "--repo", "github.com/octo/hello"]), None, origin), repo("octo", "hello"));
        assert_eq!(repo_of_gh_command(&args(&["pr", "list", "--repo=octo/hello"]), None, origin), repo("octo", "hello"));
        assert_eq!(repo_of_gh_command(&args(&["pr", "list"]), Some("env/repo"), origin), repo("env", "repo"));
        assert_eq!(repo_of_gh_command(&args(&["api", "repos/octo/hello/pulls"]), None, origin), repo("octo", "hello"));
        // gh fills the placeholders from the working directory's repository.
        assert_eq!(repo_of_gh_command(&args(&["api", "repos/{owner}/{repo}/issues"]), None, origin), repo("home", "checkout"));
        // A literal endpoint beats GH_REPO; the placeholders follow it.
        assert_eq!(repo_of_gh_command(&args(&["api", "repos/octo/second/issues"]), Some("octo/first"), origin), repo("octo", "second"));
        assert_eq!(repo_of_gh_command(&args(&["api", "repos/{owner}/{repo}/issues"]), Some("octo/first"), origin), repo("octo", "first"));
        // A URL in a flag's value (a body that links elsewhere) is not the target.
        assert_eq!(repo_of_gh_command(&args(&["pr", "create", "--body", "https://github.com/other/private/issues/1"]), None, origin), repo("home", "checkout"));
        // A URL or a repository on the command line is the target, not the checkout.
        assert_eq!(repo_of_gh_command(&args(&["pr", "view", "https://github.com/other/private/pull/1"]), None, origin), repo("other", "private"));
        assert_eq!(repo_of_gh_command(&args(&["repo", "clone", "other/private", "dir"]), None, || None), repo("other", "private"));
        assert_eq!(repo_of_gh_command(&args(&["repo", "view", "--json", "name", "o/r"]), None, origin), repo("o", "r"));
        // The repository set with `gh repo set-default` is a spec, not a URL.
        assert_eq!(repo_of_gh_command(&args(&["pr", "create"]), None, || Some("up/stream".to_string())), repo("up", "stream"));
        assert_eq!(repo_of_gh_command(&args(&["api", "/repos/octo/hello/issues", "-f", "title=x"]), None, origin), repo("octo", "hello"));
        // A repository on another host isn't ours to give a token for.
        assert_eq!(repo_of_gh_command(&args(&["pr", "list", "-R", "ghe.example.com/octo/hello"]), None, origin), None);
    }

    #[test]
    fn gh_commands_that_name_no_repository_get_no_token_and_no_noise() {
        let origin = || Some("https://github.com/home/checkout.git".to_string());
        for a in [&["--version"][..], &["auth", "status"], &["config", "list"], &["search", "repos", "x"], &[]] {
            assert_eq!(repo_of_gh_command(&args(a), None, origin), None, "{a:?}");
        }
        // Not in a git checkout: no origin, no token.
        assert_eq!(repo_of_gh_command(&args(&["pr", "list"]), None, || None), None);
    }
}
