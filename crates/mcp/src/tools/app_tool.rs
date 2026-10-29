// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App: this AgentMux itself (docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md
//! §5.1, phase 4). `version` says what is running and whether a newer
//! release exists; `upgrade` asks for it. A tool of its own, like Stop,
//! because an upgrade restarts every agent: a harness can gate it
//! (SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.1).

use agentmux_common::api_types::RequestUpgradeRequest;

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "App" => match str_arg(arguments, "action").as_deref() {
            Some("version") => version(cx).await,
            Some("upgrade") => upgrade(arguments, cx).await,
            Some("network") => network(cx).await,
            Some("history_reclaim") => history_reclaim(arguments, cx).await,
            other => anyhow::bail!("App: unknown action {other:?}; use \"version\", \"upgrade\", \"network\" or \"history_reclaim\""),
        },
        _ => Err(not_in_family()),
    }
}

async fn network(cx: &ToolCtx<'_>) -> Result<String> {
    let url = format!("{}/api/v1/app/connectivity", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .get(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App network: srv answered {status}: {body}");
    }
    Ok(describe_network(&body, agentmux_common::time::now_ms()))
}

/// `network`'s answer, from srv's `ConnectivityStatus`: one line per service, then what to make of it.
fn describe_network(status: &Value, now_ms: i64) -> String {
    let services = status["services"].as_array().map(Vec::as_slice).unwrap_or_default();
    if services.is_empty() {
        return "AgentMux hasn't checked the network yet.".to_string();
    }
    let mut out = String::from("What AgentMux can reach right now (its own checks, not your shell's):");
    let mut down = Vec::new();
    for svc in services {
        let label = s(svc, "label");
        let line = match s(svc, "state") {
            "online" => "online".to_string(),
            "unknown" => "not checked yet".to_string(),
            state => {
                down.push(label);
                let error = s(svc, "last_error");
                let why = if error.is_empty() { String::new() } else { format!(" ({error})") };
                format!("{} for {}{why}", if state == "degraded" { "degraded" } else { "OFFLINE" }, ago(now_ms, &svc["since_ms"]))
            }
        };
        out.push_str(&format!("\n- {label}: {line}"));
    }
    if down.is_empty() {
        out.push_str(
            "\nNothing AgentMux has checked is down. It tracks only the services listed, and an online service is not re-probed; \
             \"not checked yet\" and any service not listed are unknown, so this does not rule the network out.",
        );
    } else {
        out.push_str(&format!(
            "\nDown or failing: {}. A turn that stalls or retries on one of these may be the network; AgentMux keeps checking it and \
             this changes when it answers. A service not listed here is not tracked.",
            down.join(", ")
        ));
    }
    out
}

/// "5 min" or "2 h 10 min" for the time from `since_ms` to `now_ms`.
fn ago(now_ms: i64, since_ms: &Value) -> String {
    let Some(since) = since_ms.as_i64() else { return "an unknown time".to_string() };
    let secs = ((now_ms - since) / 1000).max(0);
    match secs {
        0..=89 => format!("{secs} s"),
        90..=5399 => format!("{} min", (secs + 30) / 60),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
    }
}

async fn version(cx: &ToolCtx<'_>) -> Result<String> {
    // `check=1`: ask GitHub now (srv limits it to once every 30 seconds).
    let url = format!("{}/api/v1/app/update-status?check=1", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .get(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App version: srv answered {status}: {body}");
    }
    Ok(describe_status(&body))
}

async fn upgrade(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let reason = str_arg(arguments, "reason").ok_or_else(|| anyhow::anyhow!("App upgrade needs a reason"))?;
    let request = RequestUpgradeRequest {
        auth: sign_ui_automation_auth()?,
        reason,
        user_instruction: str_arg(arguments, "user_instruction").unwrap_or_default(),
        version: str_arg(arguments, "version"),
    };
    let url = format!("{}/api/v1/agent/self/upgrade", cx.local_url.trim_end_matches('/'));
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
        anyhow::bail!("App upgrade: srv answered {status}: {}", body["error"].as_str().unwrap_or(&body.to_string()));
    }
    Ok(describe_outcome(&body))
}

/// `history_reclaim`: ask for the shared pane-history store's free space to be given back at the next start,
/// withdraw that, or read where it stands (`/api/v1/agent/history/reclaim`). srv records the request as this agent.
async fn history_reclaim(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let what = str_arg(arguments, "reclaim").unwrap_or_else(|| "status".to_string());
    let url = format!("{}/api/v1/agent/history/reclaim", cx.local_url.trim_end_matches('/'));
    let req = match what.as_str() {
        "status" => cx.client.get(&url),
        "request" | "cancel" => cx.client.post(&url).json(&json!({ "action": what })),
        other => anyhow::bail!("App history_reclaim: reclaim is \"request\", \"cancel\" or \"status\", not {other:?}"),
    };
    let resp = req.header(AUTH_KEY_HEADER, cx.auth_key).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App history_reclaim: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(describe_reclaim(&body, &what))
}

/// `history_reclaim`'s answer, from srv's status of the request and the last reclaim.
fn describe_reclaim(st: &Value, what: &str) -> String {
    let mut out = String::new();
    if st["pending"].as_bool() == Some(true) {
        let by = s(st, "requested_by");
        out.push_str(&format!(
            "A reclaim of the pane-history store is asked for{}. It runs at the next start of AgentMux, before the store opens: \
             the store is rewritten without its free pages, which takes a while for a large store and needs free disk of about \
             the store's live size plus 1 GB. The person sees it pending in Warden > History > Storage and can cancel it there; \
             App history_reclaim with reclaim \"cancel\" withdraws it too.",
            if by.is_empty() { String::new() } else { format!(" (by {by})") }
        ));
        if what == "request" && !by.is_empty() {
            out.push_str(" Asking again keeps the first request.");
        }
        let waiting = s(st, "waiting_for");
        if !waiting.is_empty() {
            out.push_str(&format!(" The last start did not run it: {waiting}."));
        }
    } else if what == "cancel" {
        out.push_str("No reclaim of the pane-history store is asked for now.");
        let was = s(st, "withdrew_request_of");
        if !was.is_empty() {
            out.push_str(&format!(" It withdrew the request made by {was}."));
        }
    } else {
        out.push_str("No reclaim of the pane-history store is asked for. App history_reclaim with reclaim \"request\" asks for one at the next start.");
    }
    let error = s(st, "last_error");
    if !error.is_empty() {
        out.push_str(&format!(" The last attempt failed: {error}."));
    }
    if st["last_done_ms"].as_u64().unwrap_or(0) > 0 {
        let (before, after) = (st["last_before_bytes"].as_u64().unwrap_or(0), st["last_after_bytes"].as_u64().unwrap_or(0));
        out.push_str(&format!(" The last reclaim took the store from {} to {}.", mb(before), mb(after)));
    }
    out
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// `key`'s value, if it is a non-empty string.
fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// `version`'s answer, from srv's `AppUpdateStatus`.
fn describe_status(st: &Value) -> String {
    let current = s(st, "current");
    let install = s(st, "install");
    let latest = s(st, "latest");
    let mut out = format!("AgentMux {current} ({install}).");
    match s(st, "state") {
        "available" => out.push_str(&format!(" A newer release is out: {latest}.")),
        "up-to-date" => out.push_str(" This is the latest release."),
        "local-build" => out.push_str(&format!(" A development build (`task dev`), which doesn't upgrade itself; the latest release is {latest}.")),
        _ => out.push_str(" Couldn't learn which release is the latest (GitHub didn't answer)."),
    }
    if s(st, "state") == "available" {
        let d = &st["download"];
        if s(d, "version") == latest {
            match s(d, "state") {
                "downloading" | "unpacking" => out.push_str(" It is downloading now."),
                "ready" => out.push_str(" It is downloaded and ready to restart into."),
                "waiting" => out.push_str(" A restart into it is waiting for agents to finish their turn."),
                "restarting" => out.push_str(" AgentMux is asking the user to confirm the restart."),
                "failed" => out.push_str(&format!(" Its download failed: {}.", s(d, "error"))),
                _ => {}
            }
        }
        let first = s(st, "install_first");
        if !first.is_empty() {
            out.push_str(&format!(
                " This version is older than {latest} upgrades from: the user installs {first} first \
                 (https://github.com/agentmuxai/agentmux/releases/tag/v{first}), then upgrades."
            ));
        } else if st["can_download"].as_bool() == Some(true) {
            out.push_str(" This install can upgrade itself: App with action \"upgrade\" asks for it.");
        } else {
            out.push_str(&format!(
                " This install can't upgrade itself yet; the user can download it from {}.",
                s(st, "url")
            ));
        }
    }
    out
}

/// `upgrade`'s answer, from srv's outcome.
fn describe_outcome(o: &Value) -> String {
    let version = s(o, "version");
    match s(o, "status") {
        "started" => {
            let mut text = format!(
                "Upgrade to {version} started: AgentMux downloads it if needed, asks the user to confirm in its own \
                 dialog, then restarts once every agent has finished its turn. End your turn so you don't hold it up. \
                 Your session resumes after the restart."
            );
            let stopping: Vec<&str> = o["stopping"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
            if !stopping.is_empty() {
                text.push_str(&format!(
                    " The restart also stops what agents left running in the background: {}. Each agent is told afterwards.",
                    stopping.join("; ")
                ));
            }
            text
        }
        "requested" => format!(
            "Asked for the upgrade to {version} as a sensitive operation ({}). If the super agent approves, the person \
             sees a banner with a countdown and can veto it, and the upgrade starts when it ends. You'll get [AgentMux] \
             notes as it goes; don't ask again meanwhile. Check it with Sensitive status, id {}.",
            s(o, "review"),
            s(o, "id")
        ),
        "up_to_date" => format!("AgentMux {} is the latest release; nothing to upgrade.", s(o, "current")),
        "unavailable" => {
            let mut out = format!("Can't upgrade: {}.", s(o, "message"));
            if let Some(url) = o["url"].as_str() {
                out.push_str(&format!(" The release page: {url}"));
            }
            out
        }
        other => format!("App upgrade: unexpected answer {other:?}: {o}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn version_says_what_runs_and_what_the_agent_can_do() {
        let st = json!({
            "state": "available", "current": "0.59.26", "latest": "0.59.27", "install": "windows-portable",
            "can_download": true, "url": "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27",
            "download": { "version": "0.59.27", "state": "ready", "received": 1, "total": 1, "waiting_for": 0 },
            "checked_ms": 1
        });
        let text = describe_status(&st);
        assert!(text.starts_with("AgentMux 0.59.26 (windows-portable). A newer release is out: 0.59.27."), "{text}");
        assert!(text.contains("downloaded and ready"), "{text}");
        assert!(text.contains("action \"upgrade\""), "{text}");

        let setup = json!({ "state": "available", "current": "0.59.26", "latest": "0.59.27", "install": "windows-setup",
            "can_download": false, "url": "https://example/r", "checked_ms": 1 });
        assert!(describe_status(&setup).contains("can't upgrade itself yet; the user can download it from https://example/r"));
        let same = json!({ "state": "up-to-date", "current": "0.59.27", "latest": "0.59.27", "install": "windows-portable", "checked_ms": 1 });
        assert!(describe_status(&same).ends_with("This is the latest release."));
        let old = json!({ "state": "available", "current": "0.57.3", "latest": "0.60.0", "install": "windows-portable",
            "can_download": false, "install_first": "0.58.0", "checked_ms": 1 });
        let text = describe_status(&old);
        assert!(text.contains("older than 0.60.0 upgrades from: the user installs 0.58.0 first"), "{text}");
        assert!(!text.contains("action \"upgrade\""), "{text}");
    }

    #[test]
    fn history_reclaim_says_who_asked_when_it_runs_and_how_to_take_it_back() {
        let pending = json!({ "pending": true, "requested_by": "loap-3", "requested_at_ms": 1, "last_done_ms": 0, "last_before_bytes": 0, "last_after_bytes": 0 });
        let text = describe_reclaim(&pending, "request");
        assert!(text.contains("is asked for (by loap-3)"), "{text}");
        assert!(text.contains("next start of AgentMux") && text.contains("Warden > History > Storage"), "{text}");
        assert!(text.contains("reclaim \"cancel\"") && text.contains("keeps the first request"), "{text}");
        assert!(!describe_reclaim(&pending, "status").contains("keeps the first request"));

        let none = json!({ "pending": false, "last_done_ms": 0, "last_before_bytes": 0, "last_after_bytes": 0 });
        assert!(describe_reclaim(&none, "cancel").starts_with("No reclaim of the pane-history store is asked for now."));
        assert!(describe_reclaim(&none, "status").contains("reclaim \"request\" asks for one"));
        let taken = json!({ "pending": false, "withdrew_request_of": "operator" });
        assert!(describe_reclaim(&taken, "cancel").contains("withdrew the request made by operator"));

        let done = json!({ "pending": false, "last_done_ms": 5, "last_before_bytes": 3u64 << 30, "last_after_bytes": 1u64 << 30 });
        assert!(describe_reclaim(&done, "status").contains("from 3072.0 MB to 1024.0 MB"));
        let failed = json!({ "pending": true, "requested_by": "operator", "waiting_for": "another AgentMux is running", "last_error": "disk full" });
        let text = describe_reclaim(&failed, "status");
        assert!(text.contains("did not run it: another AgentMux is running") && text.contains("failed: disk full"), "{text}");
    }

    #[test]
    fn network_names_each_service_and_what_is_down() {
        let now = 10_000_000;
        let all_up = json!({ "revision": 1, "services": [
            { "service": "internet", "label": "Internet", "state": "online", "since_ms": 1, "last_error": null },
            { "service": "model_api:anthropic", "label": "Anthropic API", "state": "unknown", "since_ms": 1, "last_error": null },
        ] });
        let text = describe_network(&all_up, now);
        assert!(text.contains("- Internet: online") && text.contains("- Anthropic API: not checked yet"), "{text}");
        assert!(text.contains("Nothing AgentMux has checked is down") && text.contains("does not rule the network out"), "{text}");

        let down = json!({ "revision": 2, "services": [
            { "service": "internet", "label": "Internet", "state": "online", "since_ms": 1, "last_error": null },
            { "service": "muxbus", "label": "MuxBus", "state": "offline", "since_ms": now - 7_500_000, "last_error": "connection reset" },
            { "service": "model_api:anthropic", "label": "Anthropic API", "state": "degraded", "since_ms": now - 30_000, "last_error": null },
        ] });
        let text = describe_network(&down, now);
        assert!(text.contains("- MuxBus: OFFLINE for 2 h 5 min (connection reset)"), "{text}");
        assert!(text.contains("- Anthropic API: degraded for 30 s"), "{text}");
        assert!(text.contains("Down or failing: MuxBus, Anthropic API"), "{text}");
        assert!(!text.contains("Nothing AgentMux has checked is down") && text.contains("may be the network"), "{text}");

        assert_eq!(describe_network(&json!({ "services": [] }), now), "AgentMux hasn't checked the network yet.");
    }

    #[test]
    fn every_outcome_tells_the_agent_what_happens_next() {
        let started = describe_outcome(&json!({ "status": "started", "version": "0.59.27" }));
        assert!(started.contains("End your turn") && !started.contains("background"), "{started}");
        let stopping = describe_outcome(&json!({
            "status": "started",
            "version": "0.59.27",
            "stopping": ["Korp: an AgentMux instance (`task dev`), running 9 h"],
        }));
        assert!(stopping.contains("also stops what agents left running in the background: Korp: an AgentMux instance"), "{stopping}");
        let r = describe_outcome(&json!({ "status": "requested", "version": "0.59.27", "id": "r7", "review": "the super agent has been asked" }));
        assert!(r.contains("sensitive operation (the super agent has been asked)") && r.contains("veto") && r.contains("id r7"), "{r}");
        assert!(describe_outcome(&json!({ "status": "up_to_date", "current": "0.59.27" })).contains("latest release"));
        let u = describe_outcome(&json!({ "status": "unavailable", "message": "no", "url": "https://x" }));
        assert_eq!(u, "Can't upgrade: no. The release page: https://x");
    }
}
