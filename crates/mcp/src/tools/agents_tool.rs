// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents: one tool for the other agents (who is reachable, what they are
//! doing, opening them) and the caller's own history, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S4).
//! It replaces DiscoverAgents, FleetList (the same call), ListConversations,
//! GetAgentTranscript, OpenAgent and SearchHistory; their handlers
//! (fleet.rs, history.rs, panes.rs) run each action unchanged.
//! `spawn` (the agent-initiated action that makes a worker, not the CLI's
//! process start) is its own (SPEC_AGENT_WORKERS_2026_10_15.md §3.1): srv
//! launches the worker, then this process sends it the brief, signed as the
//! caller.

use agentmux_common::api_types::SpawnAgentRequest;

use super::family::{pick, public_params, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "list", tool: "DiscoverAgents", rename: &[] },
    Act { action: "conversations", tool: "ListConversations", rename: &[] },
    Act { action: "transcript", tool: "GetAgentTranscript", rename: &[] },
    Act { action: "open", tool: "OpenAgent", rename: &[("agent", "agent_id")] },
    Act { action: "spawn", tool: "SpawnAgent", rename: &[] },
    Act { action: "workers", tool: "AgentWorkers", rename: &[] },
    Act { action: "start", tool: "AgentStart", rename: &[] },
    Act { action: "stop", tool: "AgentStop", rename: &[] },
    Act { action: "detach", tool: "AgentDetach", rename: &[] },
    Act { action: "watch", tool: "AgentWatch", rename: &[] },
    Act { action: "search_history", tool: "SearchHistory", rename: &[] },
    Act { action: "context", tool: "AgentContext", rename: &[] },
    Act { action: "compact", tool: "AgentCompact", rename: &[] },
    Act { action: "fresh", tool: "AgentFresh", rename: &[] },
    Act { action: "env_get", tool: "AgentEnvGet", rename: &[] },
    Act { action: "env_set", tool: "AgentEnvSet", rename: &[] },
    Act { action: "set_seat", tool: "AgentSetSeat", rename: &[] },
    Act { action: "update_container", tool: "AgentUpdateContainer", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Agents" => {
            let (act, args) = pick("Agents", ACTS, arguments)?;
            // Your own history only (see AGENTS_TOOL): another agent's goes
            // through the transcript_request rules, never through a search.
            if act.action == "search_history" && args.get("agent").is_some() {
                anyhow::bail!(
                    "Agents search_history searches only your own history and takes no agent; \
                     another agent's conversation is shared through its own visibility setting"
                );
            }
            let result = match act.tool {
                "DiscoverAgents" => fleet::call(act.tool, &args, cx).await,
                "OpenAgent" => panes::call(act.tool, &args, cx).await,
                "SpawnAgent" => spawn(&args, cx).await,
                "AgentWorkers" => workers(cx).await,
                "AgentStart" => lifecycle(cx, "start", &args).await,
                "AgentStop" => lifecycle(cx, "stop", &args).await,
                "AgentDetach" => lifecycle(cx, "detach", &args).await,
                "AgentWatch" => watch(cx, &args).await,
                "AgentContext" => own_context(cx, "context", None).await,
                "AgentCompact" => own_context(cx, "compact", str_arg(&args, "keep")).await,
                "AgentFresh" => match str_arg(&args, "handoff") {
                    Some(handoff) => own_context(cx, "fresh", Some(handoff)).await,
                    None => Err(anyhow::anyhow!("Agents fresh needs a handoff: what your next session must know")),
                },
                "AgentEnvGet" => env(cx, "get", &args).await,
                "AgentEnvSet" => env(cx, "set", &args).await,
                "AgentSetSeat" => set_seats(cx, &args).await,
                "AgentUpdateContainer" => update_container(cx, &args).await,
                _ => history::call(act.tool, &args, cx).await,
            };
            let result = result.map_err(|e| anyhow::anyhow!(public_params(&format!("{e:#}"), act.rename)));
            relabeled("Agents", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

/// `key`'s value, if it is a non-empty string.
fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// How long `spawn` waits for srv: the user's approval window (srv's
/// `ask_user`, 120 s and 15 s of grace), then an `agent.open` that may
/// install the worker's CLI (150 s), with room to spare. A wait that ran out
/// while srv carried on would launch a worker that never gets its brief.
const SPAWN_WAIT: std::time::Duration = std::time::Duration::from_secs(360);

/// `spawn`: srv builds a worker from this agent's definition and opens it next to it
/// (`/api/v1/agent/spawn`), then the brief goes to the worker as this
/// agent's own message, signed with its key, so it arrives host-verified
/// and the worker can answer with Message send.
async fn spawn(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let brief = str_arg(arguments, "brief").ok_or_else(|| anyhow::anyhow!("missing required parameter: brief"))?;
    let request = SpawnAgentRequest {
        auth: sign_ui_automation_auth()?,
        name: str_arg(arguments, "name"),
        model: str_arg(arguments, "model"),
        effort: str_arg(arguments, "effort"),
        brief: brief.clone(),
        account: str_arg(arguments, "account"),
        user_instruction: str_arg(arguments, "user_instruction").unwrap_or_default(),
    };
    let url = format!("{}/api/v1/agent/spawn", cx.local_url.trim_end_matches('/'));
    // Past the shared client's 10 s: srv may wait on the user's answer and
    // then on the worker's CLI install (SPAWN_WAIT).
    let resp = cx
        .client
        .post(&url)
        .timeout(SPAWN_WAIT)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let mut body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("SpawnAgent: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    let to = body["message_to"].as_str().unwrap_or_default().to_string();
    let leader = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
    let worker = body["name"].as_str().unwrap_or_default();
    let sent = super::fleet::call("SendMessage", &json!({ "to": to, "message": brief_message(&leader, worker, &to, &brief) }), cx).await;
    body["brief"] = json!(match sent {
        Ok(outcome) => outcome,
        Err(e) => format!("NOT SENT: {e:#}. The worker is open but has no task: send it the brief with Message send"),
    });
    Ok(serde_json::to_string_pretty(&body)?)
}

/// The worker's first message: who it is, who it reports to, then the leader's
/// brief. It names the tool exactly: a worker that went looking for
/// "SendMessage" found Claude Code's own cross-session tool, whose message
/// reaches the leader unverified and isn't counted as a report.
///
/// It also says whose name goes on what the worker publishes. A worker's own
/// name is in its environment, but the only name in the text it is handed is
/// its leader's, and a file it reads may show another agent's name as an
/// example (agentmux#5209: a worker started under another agent's workspace
/// titled its PR with that agent's name, and the review notices went there).
/// `gh-agent` rewrites a wrong signature at the PR; this keeps the worker from
/// writing it, in commits and comments too. `worker` is the worker's name as
/// srv made it; `slug` is its address, its `AGENTMUX_AGENT_ID`.
fn brief_message(leader: &str, worker: &str, slug: &str, brief: &str) -> String {
    let me = agentmux_common::slug::agent_route_name(worker);
    format!(
        "You are {worker} (AGENTMUX_AGENT_ID {slug}), a worker of {leader}, started to do the task below. Report to \
         {leader} with the agentmux MCP server's Message tool (mcp__agentmux__Message in Claude Code; action \
         \"send\", to \"{leader}\"), never a built-in SendMessage, when you finish, get stuck, or need a decision.\n\n\
         Sign what you publish as yourself, {me}, never as {leader}: a PR title starts \"{me}@<host>: \", its \
         \"Agent:\" line reads \"Agent: {me}@<host>\", and the agent_id tag in its body is {slug}. Where a file you \
         read shows another agent's name as an example, use yours.\n\n{brief}"
    )
}

/// `workers`: this agent's own workers, from srv (`/api/v1/agent/workers`).
async fn workers(cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let request = agentmux_common::api_types::AgentWorkersRequest { auth: sign_ui_automation_auth()? };
    let url = format!("{}/api/v1/agent/workers", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("AgentWorkers: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

/// `context` and `compact`: the caller's own pane only, which srv resolves
/// from this process's agent token (`X-Agent-Token`, sent on every request),
/// never from a name (SPEC_AGENT_SELF_COMPACTION_2026_10_23.md §3.3).
async fn own_context(cx: &ToolCtx<'_>, route: &str, focus: Option<String>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let url = format!("{}/api/v1/agent/self/{route}", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({ "agent": crate::identity::agent_slug()?, "focus": focus }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Agents {route}: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

/// What `env_get` and `env_set` send srv (`/api/v1/agent/env`): the definition
/// (the caller's own when none is named) and, for a set, the whole text or the
/// variables to set and unset. `lines` goes as written, not trimmed. A number
/// or a boolean given as a value is sent as its text.
fn env_request(action: &str, args: &Value) -> Result<Value> {
    let mut set = None;
    if let Some(given) = args.get("set").filter(|v| !v.is_null()) {
        let Some(object) = given.as_object() else {
            anyhow::bail!("Agents env_set: set is an object of NAME: value");
        };
        let mut texts = serde_json::Map::new();
        for (name, value) in object {
            let text = match value {
                Value::String(t) => t.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => anyhow::bail!("Agents env_set: the value of {name} must be text"),
            };
            texts.insert(name.clone(), Value::String(text));
        }
        set = Some(Value::Object(texts));
    }
    Ok(json!({
        "action": action,
        "agent": str_arg(args, "agent"),
        "lines": args.get("lines").and_then(Value::as_str),
        "set": set,
        "unset": args.get("unset").filter(|v| !v.is_null()),
    }))
}

/// `update_container`: srv removes the named container agent's container, so its next message
/// starts it from the current agent image. Refused while that agent has a turn running.
async fn update_container(cx: &ToolCtx<'_>, args: &Value) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let Some(agent) = str_arg(args, "agent") else {
        anyhow::bail!("Agents update_container needs agent: the container agent's id or slug");
    };
    let url = format!("{}/api/v1/agent/container/update", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({ "agent": agent }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Agents update_container: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

/// `set_seat`: seats set or cleared by srv as the agent this process's token
/// names, which records each change under that agent (`/api/v1/agent/seats`).
/// `seats` goes as given; srv checks the shape and every seat.
async fn set_seats(cx: &ToolCtx<'_>, args: &Value) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let seats = args.get("seats").filter(|v| v.is_array()).ok_or_else(|| {
        anyhow::anyhow!("Agents set_seat needs seats: [{{agent, seat}}], seat null to clear an agent's seat")
    })?;
    let url = format!("{}/api/v1/agent/seats", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({ "seats": seats }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Agents set_seat: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

/// `start`, `stop` and `detach`: srv acts on the agent in `agent` as the agent this process's token
/// names, and records it under that agent (`/api/v1/agent/{start,stop,detach}`).
async fn lifecycle(cx: &ToolCtx<'_>, action: &str, args: &Value) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let agent = str_arg(args, "agent").ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;
    let url = format!("{}/api/v1/agent/{action}", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({ "agent": agent }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Agent{}: {}", capitalised(action), body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

/// `watch`: the recent transcript of an agent, then what it wrote since `after` (`/api/v1/agent/watch`).
/// With `wait_seconds` srv holds the call until a line arrives, so the client waits a little longer.
async fn watch(cx: &ToolCtx<'_>, args: &Value) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let agent = str_arg(args, "agent").ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;
    let wait = args.get("wait_seconds").and_then(Value::as_u64).unwrap_or(0).min(25);
    let url = format!("{}/api/v1/agent/watch", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .timeout(std::time::Duration::from_secs(wait + 15))
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({
            "agent": agent,
            "max_lines": args.get("max_lines").and_then(Value::as_u64),
            "after": args.get("after").and_then(Value::as_u64),
            "wait_seconds": wait,
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("AgentWatch: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// `env_get` and `env_set`: a definition's extra environment variables, read or
/// changed by srv as the agent this process's token names, which records the
/// change under that agent (`/api/v1/agent/env`).
async fn env(cx: &ToolCtx<'_>, action: &str, args: &Value) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let request = env_request(action, args)?;
    let url = format!("{}/api/v1/agent/env", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Agents env_{action}: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::AGENTS_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let mut want = super::super::family::action_names(ACTS);
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }

    #[test]
    fn start_stop_and_detach_name_the_agent_and_run_their_own_handlers() {
        for (action, tool) in [("start", "AgentStart"), ("stop", "AgentStop"), ("detach", "AgentDetach")] {
            let (act, args) = pick("Agents", ACTS, &json!({ "action": action, "agent": "korp" })).unwrap();
            assert_eq!((act.tool, args), (tool, json!({ "agent": "korp" })));
        }
        assert_eq!(capitalised("start"), "Start");
    }

    #[test]
    fn watch_passes_its_cursor_and_wait_to_its_handler() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "watch", "agent": "korp", "after": 42, "wait_seconds": 10 })).unwrap();
        assert_eq!((act.tool, args), ("AgentWatch", json!({ "agent": "korp", "after": 42, "wait_seconds": 10 })));
    }

    #[tokio::test]
    async fn a_lifecycle_action_needs_an_agent_to_act_on() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "http://127.0.0.1:1", auth_key: "k", block_id: "b", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Agents", &json!({ "action": "stop" }), &cx).await.unwrap_err().to_string();
        assert!(e.contains("missing required parameter: agent"), "{e}");
    }

    #[test]
    fn update_container_names_the_agent_and_needs_one() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "update_container", "agent": "Korp" })).unwrap();
        assert_eq!(act.tool, "AgentUpdateContainer");
        assert_eq!(str_arg(&args, "agent").as_deref(), Some("Korp"));
        assert_eq!(str_arg(&json!({}), "agent"), None);
    }

    #[test]
    fn open_takes_the_agent_as_its_handler_names_it() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "open", "agent": "Korp", "focus": false })).unwrap();
        assert_eq!(act.tool, "OpenAgent");
        assert_eq!(args, json!({ "agent_id": "Korp", "focus": false }));
        let (_, args) = pick("Agents", ACTS, &json!({ "action": "transcript", "agent": "Korp" })).unwrap();
        assert_eq!(args, json!({ "agent": "Korp" }));
    }

    #[test]
    fn the_brief_tells_the_worker_who_it_reports_to() {
        let m = brief_message("loap-3", "Loap-4", "loap-4", "Implement SPEC_X.");
        assert!(
            m.starts_with(
                "You are Loap-4 (AGENTMUX_AGENT_ID loap-4), a worker of loap-3, started to do the task below. Report to \
                 loap-3 with the agentmux MCP server's Message tool (mcp__agentmux__Message in Claude Code; action \"send\", \
                 to \"loap-3\"), never a built-in SendMessage, when you finish, get stuck, or need a decision."
            ),
            "{m}"
        );
        assert!(m.ends_with("\n\nImplement SPEC_X."), "{m}");
        assert!(!m.contains("  "), "{m}");
    }

    /// agentmux#5209: a worker signed its PR as its leader.
    #[test]
    fn the_brief_names_the_workers_own_signature_and_forbids_its_leaders() {
        let m = brief_message("camper", "QTimer", "qtimer", "Add the side buttons.");
        assert!(m.contains("Sign what you publish as yourself, QTimer, never as camper"), "{m}");
        assert!(m.contains("a PR title starts \"QTimer@<host>: \""), "{m}");
        assert!(m.contains("\"Agent: QTimer@<host>\""), "{m}");
        assert!(m.contains("the agent_id tag in its body is qtimer"), "{m}");
        // The leader is named as who to report to and who not to sign as, never as the worker's name.
        assert!(!m.contains("camper@"), "{m}");
        assert!(m.contains("Where a file you read shows another agent's name as an example, use yours."), "{m}");
    }

    /// A name that is not a valid id is signed in route form, as a title carries it.
    #[test]
    fn the_brief_signs_a_workers_name_in_route_form() {
        let m = brief_message("loap", "Loap #4", "loap-4", "b");
        assert!(m.contains("a PR title starts \"Loap-4@<host>: \""), "{m}");
        assert!(m.contains("You are Loap #4 (AGENTMUX_AGENT_ID loap-4)"), "{m}");
    }

    #[test]
    fn spawn_runs_its_own_handler_with_its_arguments() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "spawn", "name": "Loap-4", "model": "sonnet", "brief": "b" })).unwrap();
        assert_eq!(act.tool, "SpawnAgent");
        assert_eq!(args, json!({ "name": "Loap-4", "model": "sonnet", "brief": "b" }));
    }

    /// The search reaches only the caller's own history: an agent is refused
    /// before anything is sent.
    #[tokio::test]
    async fn search_history_takes_no_agent() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Agents", &json!({ "action": "search_history", "query": "x", "agent": "Korp" }), &cx)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("Agents search_history searches only your own history"), "{e}");
    }

    #[test]
    fn env_actions_run_their_handlers() {
        for (action, tool) in [("env_get", "AgentEnvGet"), ("env_set", "AgentEnvSet")] {
            let (act, args) = pick("Agents", ACTS, &json!({ "action": action, "agent": "korp" })).unwrap();
            assert_eq!((act.tool, args), (tool, json!({ "agent": "korp" })));
        }
    }

    #[test]
    fn an_env_get_names_the_agent_or_leaves_it_to_srv() {
        assert_eq!(
            env_request("get", &json!({ "agent": " korp " })).unwrap(),
            json!({ "action": "get", "agent": "korp", "lines": null, "set": null, "unset": null })
        );
        assert_eq!(env_request("get", &json!({})).unwrap()["agent"], Value::Null);
    }

    #[test]
    fn an_env_set_sends_the_text_as_written_or_the_edits() {
        let text = "# note

EDITOR=vim  
";
        let sent = env_request("set", &json!({ "lines": text })).unwrap();
        assert_eq!(sent["lines"], text);
        assert_eq!((sent["set"].clone(), sent["unset"].clone()), (Value::Null, Value::Null));
        let sent = env_request("set", &json!({ "agent": "korp", "set": { "A": "1", "B": 2, "C": true }, "unset": ["D"] })).unwrap();
        assert_eq!(sent["set"], json!({ "A": "1", "B": "2", "C": "true" }));
        assert_eq!((sent["unset"].clone(), sent["lines"].clone(), sent["agent"].clone()), (json!(["D"]), Value::Null, json!("korp")));
        // Clearing the environment is an empty text, not a missing one.
        assert_eq!(env_request("set", &json!({ "lines": "" })).unwrap()["lines"], "");
    }

    #[test]
    fn an_env_set_value_that_is_not_text_is_refused_before_anything_is_sent() {
        let e = env_request("set", &json!({ "set": { "A": ["x"] } })).unwrap_err().to_string();
        assert_eq!(e, "Agents env_set: the value of A must be text");
        let e = env_request("set", &json!({ "set": "A=1" })).unwrap_err().to_string();
        assert_eq!(e, "Agents env_set: set is an object of NAME: value");
    }

    /// Without an agent's own environment nothing is sent: the change would have no one to be attributed to.
    #[tokio::test]
    async fn env_set_needs_an_agent_pane() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        assert!(call("Agents", &json!({ "action": "env_set", "set": { "A": "1" } }), &cx).await.is_err());
    }
}
