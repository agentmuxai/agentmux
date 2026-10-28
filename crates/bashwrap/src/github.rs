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
    if let Some(r) = explicit.as_deref().or(env_repo).filter(|r| !r.is_empty()) {
        return repo_of_spec(r);
    }
    // gh api repos/OWNER/REPO/…
    if args.first().map(String::as_str) == Some("api") {
        for a in &args[1..] {
            let p = a.trim_start_matches('/');
            if let Some(rest) = p.strip_prefix("repos/") {
                if let Some(found) = owner_repo_of_path(rest) {
                    return Some(found);
                }
            }
        }
    }
    // gh needs no repository for these.
    if matches!(args.first().map(String::as_str), None | Some("--version" | "--help" | "help" | "version" | "auth" | "config" | "completion" | "alias" | "extension" | "ext" | "gist" | "search")) {
        return None;
    }
    repo_of_remote(&origin_url()?)
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

/// The answer of `POST /api/v1/github/token`: a token, or what to tell the person.
pub enum Answer {
    Token(String),
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
            Some(token) => Answer::Token(token.to_string()),
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
        Answer::Token(token) => {
            let mut out = std::io::stdout();
            let _ = out.write_all(credential_response(&token).as_bytes());
            let _ = out.flush();
            0
        }
        Answer::Refused(why) => {
            eprintln!("agentmux: no GitHub token for {owner}/{repo}: {why}");
            1
        }
    }
}

/// `github-token -- <gh args>`: print a token for the repository the `gh` command is about; with
/// none, say why on stderr (when there is a repository) and print nothing.
pub fn run_github_token(gh_args: &[String]) -> i32 {
    let env_repo = std::env::var("GH_REPO").ok();
    let origin = || {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["config", "--get", "remote.origin.url"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // Windows: git is a console program, and this process has no console of its own.
        cmd.no_window();
        let out = cmd.output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let Some((owner, repo)) = repo_of_gh_command(gh_args, env_repo.as_deref(), origin) else {
        return 0;
    };
    match ask(&owner, &repo) {
        Answer::Token(token) => {
            println!("{token}");
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
        let (url, srv) = stub_srv(200, r#"{"token":"ghs_FROM_SRV","expiresAt":1,"botSlug":"muxagent","botUserId":7}"#);
        std::env::set_var("AGENTMUX_LOCAL_URL", &url);
        std::env::set_var("AGENTMUX_AUTH_KEY", "key");
        std::env::set_var("AGENTMUX_AGENT_TOKEN", "agent-token-1");
        let answer = ask("octo", "hello");
        let seen = srv.join().unwrap();
        assert!(matches!(&answer, Answer::Token(t) if t == "ghs_FROM_SRV"));
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
