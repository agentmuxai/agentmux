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
            let pr = match arguments.get("pr").filter(|v| !v.is_null()) {
                Some(v) => Some(pr_number(v).ok_or_else(|| anyhow::anyhow!("pr must be a PR number (12, \"#12\" or its URL)"))?),
                None => None,
            };
            let query = who_query(arguments, &agent, block_id);
            let url = format!("{}/api/v1/work-facts/who", local_url.trim_end_matches('/'));
            let mut body = srv_get_json(client, &url, auth_key, &query, "WhoIsWorkingOn").await?;
            add_open_prs(&mut body, arguments, pr, &agent, |path| {
                let mut q = query.clone();
                q.retain(|(k, _)| *k != "path");
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
        _ => Err(not_in_family()),
    }
}

/// `12`, `"12"`, `"#12"` or a PR's URL (`…/pull/12`).
fn pr_number(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n).filter(|n| *n > 0);
    }
    let s = v.as_str()?.trim().trim_end_matches('/');
    let tail = s.rsplit(['/', '#']).next()?;
    tail.parse().ok().filter(|n| *n > 0)
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
    let prs = match open_prs::open_prs(&repo).await {
        Ok(prs) => prs,
        Err(e) => {
            body["pr_error"] = json!(e);
            return;
        }
    };
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
        for file in files.iter().take(PR_FILES_CHECKED) {
            if let Some(answer) = ask_path(file.clone()).await {
                merge_agents(&mut agents, answer.get("agents").and_then(Value::as_array).cloned().unwrap_or_default());
            }
        }
        body["agents"] = Value::Array(agents);
        if files.len() > PR_FILES_CHECKED {
            body["pr_note"] = json!(format!(
                "the PR changes {} files; agents on this computer were checked against the first {PR_FILES_CHECKED}",
                files.len()
            ));
        }
    }
    open_prs::merge_into(body, matches, &host, &repo);
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
    fn a_pr_is_a_number_a_hash_number_or_a_url() {
        assert_eq!(pr_number(&json!(12)), Some(12));
        assert_eq!(pr_number(&json!("12")), Some(12));
        assert_eq!(pr_number(&json!("#12")), Some(12));
        assert_eq!(pr_number(&json!("https://github.com/o/r/pull/4683/")), Some(4683));
        assert_eq!(pr_number(&json!("main")), None);
        assert_eq!(pr_number(&json!(0)), None);
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
