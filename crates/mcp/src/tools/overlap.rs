// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who else is working on the same thing
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.2), and saying
//! what you are working on (`ClaimWork`, `ReleaseWork`, §3.4).

use super::*;

/// The query srv's `/api/v1/work-facts/who` takes for these arguments, plus
/// who is asking (srv prefers the request's agent token to both).
fn who_query(arguments: &Value, agent: &str, block_id: &str) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&'static str, String)> = Vec::new();
    for key in ["path", "repo", "branch", "query"] {
        if let Some(v) = arguments.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()) {
            q.push((key, v.to_string()));
        }
    }
    if !agent.trim().is_empty() {
        q.push(("agent", agent.trim().to_string()));
    }
    if !block_id.trim().is_empty() {
        q.push(("block", block_id.trim().to_string()));
    }
    q
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "WhoIsWorkingOn" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let (pr_repo, pr) = match arguments.get("pr").filter(|v| !v.is_null()) {
                Some(v) => {
                    let (repo, n) = pr_ref(v)
                        .ok_or_else(|| anyhow::anyhow!("pr must be a PR number (12, \"#12\") or its GitHub URL"))?;
                    (repo, Some(n))
                }
                None => (None, None),
            };
            let mut query = who_query(arguments, &agent, block_id);
            // srv's recent_posts then holds the PR's own posts.
            if let Some(n) = pr {
                query.push(("pr", n.to_string()));
            }
            // A PR's URL names its repository: ask about that one.
            if let Some(url_repo) = pr_repo {
                match arg(arguments, "repo") {
                    Some(r) if normalize_repo(r) != normalize_repo(&url_repo) => {
                        anyhow::bail!("pr is a PR of {url_repo}, but repo is {r}: pass one or the other");
                    }
                    Some(_) => {}
                    None => query.push(("repo", url_repo)),
                }
            }
            let url = format!("{}/api/v1/work-facts/who", local_url.trim_end_matches('/'));
            let mut body = srv_get_json(client, &url, auth_key, &query, "WhoIsWorkingOn").await?;
            add_open_prs(&mut body, arguments, pr, &agent, |path| {
                // Only the file, its repository and who is asking: srv
                // matches each criterion on its own, so a branch or query
                // here would list agents that touch none of the PR's files.
                let mut q = query.clone();
                q.retain(|(k, _)| matches!(*k, "repo" | "agent" | "block"));
                // Not `pr`: these ask about one of its files, whose posts are merged in.
                q.push(("path", path));
                let url = url.clone();
                async move { srv_get_json(client, &url, auth_key, &q, "WhoIsWorkingOn").await.ok() }
            })
            .await;
            Ok(serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()))
        }
        "ClaimWork" | "ReleaseWork" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let base = local_url.trim_end_matches('/');
            let (url, body) = if name == "ClaimWork" {
                (format!("{base}/api/v1/work-claims"), claim_body(arguments, &agent, block_id))
            } else {
                (format!("{base}/api/v1/work-claims/release"), release_body(arguments, &agent, block_id))
            };
            let answer = srv_post_json(client, &url, auth_key, &body, name).await?;
            Ok(serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.to_string()))
        }
        "PostToChannel" | "ReadChannel" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let url = format!("{}/api/v1/coord/posts", local_url.trim_end_matches('/'));
            let answer = if name == "PostToChannel" {
                let body = channel_post_body(arguments, &agent, block_id)?;
                srv_post_json(client, &url, auth_key, &body, name).await?
            } else {
                let query = channel_read_query(arguments, &agent, block_id)?;
                srv_get_json(client, &url, auth_key, &query, name).await?
            };
            Ok(serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.to_string()))
        }
        _ => Err(not_in_family()),
    }
}

/// A coordination post's PR argument: its number, and the repository its URL
/// names (which must agree with `repo` when both are given).
fn channel_pr(arguments: &Value) -> Result<(Option<String>, Option<u64>)> {
    let Some(v) = arguments.get("pr").filter(|v| !v.is_null()) else { return Ok((None, None)) };
    let (url_repo, n) =
        pr_ref(v).ok_or_else(|| anyhow::anyhow!("pr must be a PR number (12, \"#12\") or its GitHub URL"))?;
    let repo = match (url_repo, arg(arguments, "repo")) {
        (Some(u), Some(r)) if normalize_repo(r) != normalize_repo(&u) => {
            anyhow::bail!("pr is a PR of {u}, but repo is {r}: pass one or the other")
        }
        (Some(u), None) => Some(u),
        _ => None,
    };
    Ok((repo, Some(n)))
}

/// `PostToChannel`'s request body.
fn channel_post_body(arguments: &Value, agent: &str, block_id: &str) -> Result<Value> {
    let mut body = serde_json::Map::new();
    for key in ["message", "path", "repo", "branch", "topic"] {
        if let Some(v) = arg(arguments, key) {
            body.insert(key.into(), json!(v));
        }
    }
    let (pr_repo, pr) = channel_pr(arguments)?;
    if let Some(r) = pr_repo {
        body.insert("repo".into(), json!(r));
    }
    if let Some(n) = pr {
        body.insert("pr".into(), json!(n));
    }
    body.insert("agent".into(), json!(agent.trim()));
    body.insert("block".into(), json!(block_id.trim()));
    Ok(Value::Object(body))
}

/// `ReadChannel`'s query string.
fn channel_read_query(arguments: &Value, agent: &str, block_id: &str) -> Result<Vec<(&'static str, String)>> {
    let mut q = who_query(arguments, agent, block_id);
    let (pr_repo, pr) = channel_pr(arguments)?;
    if let Some(r) = pr_repo {
        q.push(("repo", r));
    }
    if let Some(n) = pr {
        q.push(("pr", n.to_string()));
    }
    for key in ["since_minutes", "limit"] {
        if let Some(n) = arguments.get(key).and_then(|v| v.as_u64()) {
            q.push((key, n.to_string()));
        }
    }
    Ok(q)
}

/// A PR: `12`, `"12"`, `"#12"`, or its URL
/// (`https://github.com/<owner>/<repo>/pull/12`, with anything after the
/// number, like `/files` or `#issuecomment-…`), which also names its
/// repository: `(Some("owner/repo"), 12)`.
fn pr_ref(v: &Value) -> Option<(Option<String>, u64)> {
    if let Some(n) = v.as_u64() {
        return (n > 0).then_some((None, n));
    }
    let s = v.as_str()?.trim();
    let number = |t: &str| t.parse::<u64>().ok().filter(|n| *n > 0);
    let Some((before, after)) = s.split_once("/pull/") else {
        return number(s.trim_start_matches('#')).map(|n| (None, n));
    };
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    let rest = &after[digits.len()..];
    if !(rest.is_empty() || rest.starts_with(['/', '#', '?'])) {
        return None;
    }
    let mut parts = before.rsplit('/');
    let (repo, owner) = (parts.next()?, parts.next()?);
    if repo.is_empty() || owner.is_empty() || owner.contains(':') {
        return None;
    }
    Some((Some(format!("{owner}/{repo}")), number(&digits)?))
}

/// `owner/repo`, lowercased, from `owner/repo`, a remote URL
/// (`https://github.com/o/r.git`) or an ssh remote (`git@github.com:o/r`).
fn normalize_repo(repo: &str) -> String {
    let s = repo.trim().trim_end_matches('/').trim_end_matches(".git").replace(':', "/");
    let mut parts = s.rsplit('/').filter(|p| !p.is_empty());
    match (parts.next(), parts.next()) {
        (Some(name), Some(owner)) => format!("{owner}/{name}").to_ascii_lowercase(),
        _ => s.to_ascii_lowercase(),
    }
}

/// Files of the asked PR whose local facts are fetched, at most.
const PR_FILES_CHECKED: usize = 10;

/// Add the open PRs that touch the question (`open_prs.rs`) to srv's answer,
/// for the repository srv resolved. With `pr`, the agents on this computer
/// are the ones with that PR's files changed or edited (`ask_path`, srv's
/// answer for one file), not everyone in the repository. A failure is
/// reported in the answer (`pr_error`) and never fails the call.
async fn add_open_prs<F, Fut>(body: &mut Value, arguments: &Value, pr: Option<u64>, me: &str, ask_path: F)
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    let repo = body.pointer("/asked/repo").and_then(Value::as_str).map(str::to_string);
    let Some(repo) = repo.filter(|r| r.contains('/') && !r.starts_with("local:")) else {
        if pr.is_some() {
            body["pr_error"] = json!("pr needs a repository on GitHub (owner/repo)");
        }
        return;
    };
    let mut prs = match open_prs::open_prs(&repo).await {
        Ok(prs) => prs,
        Err(e) => {
            body["pr_error"] = json!(e);
            return;
        }
    };
    // An asked PR the list didn't hold (opened since it was cached): read it.
    if let Some(n) = pr.filter(|n| !prs.iter().any(|p| p.number == *n)) {
        match open_prs::open_pr(&repo, n).await {
            Ok(Some(p)) => prs.push(p),
            Ok(None) => {}
            Err(e) => {
                body["pr_error"] = json!(e);
                return;
            }
        }
    }
    let host = open_prs::this_host();
    let path = body.pointer("/asked/path").and_then(Value::as_str).map(str::to_string);
    let asked = open_prs::Asked {
        path: path.as_deref(),
        branch: arg(arguments, "branch"),
        query: arg(arguments, "query"),
        pr,
    };
    let matches = match open_prs::pr_matches(&prs, &asked, me, &host) {
        Ok(m) => m,
        Err(e) => {
            body["pr_error"] = json!(e);
            return;
        }
    };
    if let Some(files) = pr.and_then(|n| prs.iter().find(|p| p.number == n)).map(|p| p.files.clone()) {
        let mut agents: Vec<Value> = Vec::new();
        let mut posts: Vec<Value> = body.get("recent_posts").and_then(Value::as_array).cloned().unwrap_or_default();
        for file in files.iter().take(PR_FILES_CHECKED) {
            if let Some(answer) = ask_path(file.clone()).await {
                merge_agents(&mut agents, answer.get("agents").and_then(Value::as_array).cloned().unwrap_or_default());
                merge_posts(&mut posts, answer.get("recent_posts").and_then(Value::as_array).cloned().unwrap_or_default());
            }
        }
        body["agents"] = Value::Array(agents);
        // The PR's own posts and its files' posts, newest first, as srv caps them.
        posts.sort_by_key(|p| std::cmp::Reverse(p.get("at").and_then(Value::as_i64).unwrap_or(0)));
        posts.truncate(RECENT_POSTS_MAX);
        body["recent_posts"] = Value::Array(posts);
        if files.len() > PR_FILES_CHECKED {
            body["pr_note"] = json!(format!(
                "the PR changes {} files; agents on this computer were checked against the first {PR_FILES_CHECKED}",
                files.len()
            ));
        }
    }
    open_prs::merge_into(body, matches, &host, &repo);
}

/// `WhoIsWorkingOn`'s `recent_posts` holds at most this many (srv's cap).
const RECENT_POSTS_MAX: usize = 5;

/// Add posts not already in `into` (by id).
fn merge_posts(into: &mut Vec<Value>, from: Vec<Value>) {
    for post in from {
        if !into.iter().any(|p| p.get("id") == post.get("id")) {
            into.push(post);
        }
    }
}

/// Merge srv answers for several files: one entry per agent and channel,
/// with every distinct match.
fn merge_agents(into: &mut Vec<Value>, from: Vec<Value>) {
    for agent in from {
        let key = |a: &Value| (a.get("agent").cloned(), a.get("channel").cloned());
        match into.iter_mut().find(|a| key(a) == key(&agent)) {
            Some(existing) => {
                let new = agent.get("matches").and_then(Value::as_array).cloned().unwrap_or_default();
                if let Some(list) = existing.get_mut("matches").and_then(Value::as_array_mut) {
                    for m in new {
                        if !list.contains(&m) {
                            list.push(m);
                        }
                    }
                }
            }
            None => into.push(agent),
        }
    }
}

/// The trimmed, non-empty string argument `key`.
fn arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

/// What srv's `/api/v1/work-claims` takes: the claimed target, note and
/// lifetime, plus who is asking (srv prefers the request's agent token).
fn claim_body(arguments: &Value, agent: &str, block_id: &str) -> Value {
    let mut body = serde_json::Map::new();
    for key in ["path", "repo", "branch", "topic", "note"] {
        if let Some(v) = arg(arguments, key) {
            body.insert(key.into(), json!(v));
        }
    }
    if let Some(m) = arguments.get("ttl_minutes").and_then(|v| v.as_u64()) {
        body.insert("ttl_minutes".into(), json!(m));
    }
    body.insert("agent".into(), json!(agent.trim()));
    body.insert("block".into(), json!(block_id.trim()));
    Value::Object(body)
}

/// What srv's `/api/v1/work-claims/release` takes: a claim id, or none for all.
fn release_body(arguments: &Value, agent: &str, block_id: &str) -> Value {
    json!({ "id": arg(arguments, "id"), "agent": agent.trim(), "block": block_id.trim() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_post_carries_its_pr_and_the_repository_a_pr_url_names() {
        let body = channel_post_body(
            &json!({ "message": "regenerate INDEX.md", "path": "docs/specs", "pr": "https://github.com/o/r/pull/12" }),
            "Loap-3",
            "blk",
        )
        .unwrap();
        assert_eq!(body["message"], "regenerate INDEX.md");
        assert_eq!(body["path"], "docs/specs");
        assert_eq!(body["repo"], "o/r");
        assert_eq!(body["pr"], 12);
        assert_eq!((body["agent"].as_str(), body["block"].as_str()), (Some("Loap-3"), Some("blk")));
        assert!(channel_post_body(&json!({ "message": "x", "repo": "o/other", "pr": "https://github.com/o/r/pull/12" }), "a", "b").is_err());
        assert!(channel_post_body(&json!({ "message": "x", "pr": "nope" }), "a", "b").is_err());
    }

    #[test]
    fn posts_from_several_files_merge_once_each() {
        let mut into = vec![json!({ "id": "a", "at": 3 })];
        merge_posts(&mut into, vec![json!({ "id": "a", "at": 3 }), json!({ "id": "b", "at": 5 })]);
        assert_eq!(into.iter().map(|p| p["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn a_channel_read_passes_its_filters_window_and_limit() {
        let q = channel_read_query(&json!({ "path": "a.rs", "pr": 7, "since_minutes": 60, "limit": 5, "query": "index" }), "a", "b").unwrap();
        let get = |k: &str| q.iter().find(|(key, _)| *key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("path"), Some("a.rs"));
        assert_eq!(get("query"), Some("index"));
        assert_eq!(get("pr"), Some("7"));
        assert_eq!(get("since_minutes"), Some("60"));
        assert_eq!(get("limit"), Some("5"));
    }

    #[test]
    fn a_pr_is_a_number_a_hash_number_or_a_url_naming_its_repository() {
        assert_eq!(pr_ref(&json!(12)), Some((None, 12)));
        assert_eq!(pr_ref(&json!("12")), Some((None, 12)));
        assert_eq!(pr_ref(&json!("#12")), Some((None, 12)));
        let url = |u: &str| pr_ref(&json!(u));
        assert_eq!(url("https://github.com/o/r/pull/4683"), Some((Some("o/r".into()), 4683)));
        assert_eq!(url("https://github.com/o/r/pull/4683/"), Some((Some("o/r".into()), 4683)));
        assert_eq!(url("https://github.com/o/r/pull/4683/files"), Some((Some("o/r".into()), 4683)));
        assert_eq!(url("https://github.com/o/r/pull/4683#issuecomment-1"), Some((Some("o/r".into()), 4683)));
        assert_eq!(url("https://github.com/o/r/pull/4683x"), None);
        assert_eq!(url("https://github.com/o/r/issues/12"), None);
        assert_eq!(url("main"), None);
        assert_eq!(pr_ref(&json!(0)), None);
    }

    #[test]
    fn repositories_are_compared_whole_not_by_suffix() {
        assert_eq!(normalize_repo("Foo/Bar"), "foo/bar");
        assert_eq!(normalize_repo("https://github.com/foo/bar.git"), "foo/bar");
        assert_eq!(normalize_repo("git@github.com:foo/bar"), "foo/bar");
        assert_ne!(normalize_repo("notfoo/bar"), normalize_repo("foo/bar"));
    }

    #[test]
    fn answers_for_several_files_merge_by_agent() {
        let mut agents = Vec::new();
        merge_agents(&mut agents, vec![json!({ "agent": "A", "channel": "c", "matches": [{ "kind": "dirty", "detail": "x" }] })]);
        merge_agents(&mut agents, vec![
            json!({ "agent": "A", "channel": "c", "matches": [{ "kind": "dirty", "detail": "x" }, { "kind": "edited", "detail": "y" }] }),
            json!({ "agent": "B", "channel": "c", "matches": [] }),
        ]);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["matches"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn only_given_arguments_and_the_callers_identity_are_sent() {
        let q = who_query(&json!({ "path": " crates/srv ", "query": "", "repo": null, "pr": 12 }), "AgentY", "blk");
        assert_eq!(
            q,
            vec![("path", "crates/srv".to_string()), ("agent", "AgentY".to_string()), ("block", "blk".to_string())]
        );
        assert!(who_query(&json!({}), "", "").is_empty());
    }

    #[test]
    fn claims_send_only_what_was_given_plus_the_callers_identity() {
        let body = claim_body(
            &json!({ "path": " crates/srv ", "topic": "", "note": "presence work", "ttl_minutes": 45, "repo": null }),
            "AgentY",
            "blk",
        );
        assert_eq!(
            body,
            json!({ "path": "crates/srv", "note": "presence work", "ttl_minutes": 45, "agent": "AgentY", "block": "blk" })
        );
        assert_eq!(release_body(&json!({}), "AgentY", "blk"), json!({ "id": null, "agent": "AgentY", "block": "blk" }));
        assert_eq!(release_body(&json!({ "id": "c1" }), "AgentY", "")["id"], "c1");
    }

    #[tokio::test]
    async fn without_agentmux_env_it_says_so() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        for tool in ["WhoIsWorkingOn", "ClaimWork", "ReleaseWork"] {
            let err = call(tool, &json!({ "path": "a" }), &cx).await.unwrap_err();
            assert!(err.to_string().contains("AGENTMUX_LOCAL_URL"), "{tool}");
        }
    }
}
