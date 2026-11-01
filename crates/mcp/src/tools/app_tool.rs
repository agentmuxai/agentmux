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
            Some("version") => match remote_op::host_arg(arguments) {
                Some(host) => remote_version(arguments, cx, &host).await,
                None => version(cx).await,
            },
            Some("health") => match remote_op::host_arg(arguments) {
                Some(host) => remote_health(arguments, cx, &host).await,
                None => local_health(cx).await,
            },
            Some("upgrade") => match remote_op::host_arg(arguments) {
                Some(host) => remote_upgrade(arguments, cx, &host).await,
                None => upgrade(arguments, cx).await,
            },
            Some("network") => network(cx).await,
            Some("history_reclaim") => match remote_op::host_arg(arguments) {
                Some(host) => remote_history_reclaim(arguments, cx, &host).await,
                None => history_reclaim(arguments, cx).await,
            },
            Some("logs") => logs(arguments, cx).await,
            Some("disk") => disk(arguments, cx).await,
            other => anyhow::bail!("App: unknown action {other:?}; use \"version\", \"health\", \"upgrade\", \"network\", \"history_reclaim\", \"logs\" or \"disk\""),
        },
        _ => Err(not_in_family()),
    }
}

/// `version` with a `host`: the AgentMux on that machine, from its own answer.
async fn remote_version(arguments: &Value, cx: &ToolCtx<'_>, host: &str) -> Result<String> {
    let answer = ask(arguments, cx, host, "app.version").await?;
    let r = &answer["result"];
    Ok(format!(
        "{host} runs AgentMux {} (build {}, channel {}, {}). {}",
        s(r, "version"),
        s(r, "build"),
        s(r, "channel"),
        s(r, "os"),
        describe_status(&r["update"])
    ))
}

/// `health` with a `host`: what a steward checks after an upgrade, on that machine.
async fn remote_health(arguments: &Value, cx: &ToolCtx<'_>, host: &str) -> Result<String> {
    let answer = ask(arguments, cx, host, "app.health").await?;
    Ok(remote_op::pretty(host, &answer))
}

/// `health` on this machine, from the same report a remote call gets.
async fn local_health(cx: &ToolCtx<'_>) -> Result<String> {
    let url = format!("{}/api/v1/app/health", cx.local_url.trim_end_matches('/'));
    let resp = cx.client.get(&url).header(AUTH_KEY_HEADER, cx.auth_key).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App health: srv answered {status}: {body}");
    }
    Ok(format!("This machine:
{}", serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())))
}

/// Run a read-only operation on `host`; an error carries the machine's refusal in words.
async fn ask(arguments: &Value, cx: &ToolCtx<'_>, host: &str, op: &str) -> Result<Value> {
    ask_with(arguments, cx, host, op, json!({})).await
}

/// [`ask`] with the operation's parameters.
async fn ask_with(arguments: &Value, cx: &ToolCtx<'_>, host: &str, op: &str, params: Value) -> Result<Value> {
    let reason = str_arg(arguments, "reason").unwrap_or_else(|| format!("App {}", op.trim_start_matches("app.")));
    let answer = remote_op::remote_op_call(cx, host, op, params, &reason).await?;
    match remote_op::explain_not_done(host, op, &answer) {
        Some(problem) => anyhow::bail!("{problem}"),
        None => Ok(answer),
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

/// `upgrade` with a `host`: ask that machine to upgrade itself. It is filed there as the sensitive request a local
/// agent's `upgrade` files: its super agent decides and the person at that machine can veto the countdown, so this
/// never starts anything by itself. With `request_id`, read where an earlier request stands instead.
async fn remote_upgrade(arguments: &Value, cx: &ToolCtx<'_>, host: &str) -> Result<String> {
    if let Some(id) = str_arg(arguments, "request_id") {
        let answer = ask_with(arguments, cx, host, "app.upgrade", json!({ "status": id })).await?;
        return Ok(describe_remote_request(host, &answer["result"]));
    }
    let reason = str_arg(arguments, "reason").ok_or_else(|| anyhow::anyhow!("App upgrade needs a reason"))?;
    let mut params = json!({ "reason": reason });
    if let Some(version) = str_arg(arguments, "version") {
        params["version"] = json!(version);
    }
    let answer = ask_with(arguments, cx, host, "app.upgrade", params).await?;
    let r = &answer["result"];
    if s(r, "status") == "requested" {
        return Ok(format!(
            "Asked {host} to upgrade to {} as a sensitive operation ({}). If its super agent approves, the person at {host} \
             sees a banner with a countdown and can veto it, and the upgrade starts when it ends. Nothing is sent to you when \
             it moves on: read it with App upgrade, host {host}, request_id {}.",
            s(r, "version"),
            s(r, "review"),
            s(r, "id")
        ));
    }
    Ok(format!("{host}: {}", describe_outcome(r)))
}

/// A sensitive request's state on another machine, from `app.upgrade` with a `status`.
fn describe_remote_request(host: &str, r: &Value) -> String {
    let mut out = format!("On {host}: {} is {}", s(r, "summary"), s(r, "status"));
    if !s(r, "decided_by").is_empty() {
        out.push_str(&format!(", decided by {}", s(r, "decided_by")));
    }
    if r["deadline_ms"].is_number() {
        out.push_str(" (its countdown is running; the person there can still veto it)");
    }
    if !s(r, "deny_reason").is_empty() {
        out.push_str(&format!(". Denied: {}", s(r, "deny_reason")));
    }
    if !s(r, "error").is_empty() {
        out.push_str(&format!(". It stopped with: {}", s(r, "error")));
    }
    out.push('.');
    out
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

/// `history_reclaim` with a `host`: the same request, withdrawal or read, on that machine.
async fn remote_history_reclaim(arguments: &Value, cx: &ToolCtx<'_>, host: &str) -> Result<String> {
    let what = str_arg(arguments, "reclaim").unwrap_or_else(|| "status".to_string());
    if !matches!(what.as_str(), "request" | "cancel" | "status") {
        anyhow::bail!("App history_reclaim: reclaim is \"request\", \"cancel\" or \"status\", not {what:?}");
    }
    let answer = ask_with(arguments, cx, host, "app.history_reclaim", json!({ "action": what })).await?;
    Ok(format!("On {host}: {}", describe_reclaim(&answer["result"], &what)))
}

/// `logs`: the last lines of AgentMux's own srv or host log on another machine (`app.logs.tail`).
/// Without a `host` there is nothing to fetch: your own logs are on this machine, readable with the muxlog tool.
async fn logs(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let Some(host) = remote_op::host_arg(arguments) else {
        anyhow::bail!("App logs reads another machine's AgentMux log: name it with host. Your own logs are on this machine (muxlog).");
    };
    let which = str_arg(arguments, "log").unwrap_or_else(|| "srv".to_string());
    let mut params = json!({ "which": which });
    if let Some(n) = arguments.get("lines").and_then(Value::as_u64) {
        params["lines"] = json!(n);
    }
    let answer = ask_with(arguments, cx, &host, "app.logs.tail", params).await?;
    let r = &answer["result"];
    let note = if r["shortened"].as_bool() == Some(true) { " (shortened to fit)" } else { "" };
    Ok(format!("{host}'s {} log, last {} lines{note}:
{}", s(r, "which"), r["lines"], s(r, "text")))
}

/// `disk`: what AgentMux filled on another machine's disk (`app.disk.report`), measured there and deleting nothing.
async fn disk(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let Some(host) = remote_op::host_arg(arguments) else {
        anyhow::bail!("App disk reads another machine's disk use by AgentMux: name it with host. This machine's disk is yours to look at with a shell.");
    };
    let answer = ask(arguments, cx, &host, "app.disk.report").await?;
    Ok(describe_disk(&host, &answer["result"]))
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

fn describe_volume(label: &str, v: &Value) -> Option<String> {
    let (free, total) = (v["free_bytes"].as_u64()?, v["total_bytes"].as_u64()?);
    Some(format!("{label}: {} free of {}", gb(free), gb(total)))
}

/// `disk`'s answer, from the report: volumes, the largest build folders, older versions.
fn describe_disk(host: &str, r: &Value) -> String {
    let mut out = format!("{host}'s disk, as AgentMux sees it:");
    for line in [describe_volume("data volume", &r["data_volume"]), describe_volume("agents volume", &r["agents_volume"])].into_iter().flatten() {
        out.push_str(&format!("\n- {line}"));
    }
    let folders = r["build_folders"].as_array().map(Vec::as_slice).unwrap_or_default();
    out.push_str(&format!("\nBuild folders (cargo target): {} in all", gb(r["build_folders_total_bytes"].as_u64().unwrap_or(0))));
    for f in folders {
        let cut = if f["complete"].as_bool() == Some(false) { "+" } else { "" };
        out.push_str(&format!("\n- {}: {}{cut} (incremental {})", s(f, "clone"), gb(f["target_bytes"].as_u64().unwrap_or(0)), gb(f["incremental_bytes"].as_u64().unwrap_or(0))));
    }
    let versions = r["older_versions"].as_array().map(Vec::as_slice).unwrap_or_default();
    if !versions.is_empty() {
        out.push_str("\nOlder version folders:");
        for v in versions {
            let cut = if v["complete"].as_bool() == Some(false) { "+" } else { "" };
            out.push_str(&format!("\n- {}: {}{cut}", s(v, "version"), gb(v["bytes"].as_u64().unwrap_or(0))));
        }
    }
    if r["complete"].as_bool() == Some(false) {
        out.push_str("\nIt stopped measuring early: the sizes marked + and the totals are lower bounds.");
    }
    out.push_str(&format!("\n{}", s(r, "note")));
    out
}

/// `history_reclaim`'s answer, from srv's status of the request and the last reclaim.
fn describe_reclaim(st: &Value, what: &str) -> String {
    let mut out = String::new();
    if st["pending"].as_bool() == Some(true) {
        let by = s(st, "requested_by");
        out.push_str(&format!(
            "A reclaim of the pane-history store is asked for{}. It runs at the next start of AgentMux, before the store opens: \
             the store is rewritten without its free pages, which takes a while for a large store and needs free disk of about \
             the store's live size plus 1 GB. The person sees it pending in History > Storage and can cancel it there; \
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
                // A remote answer carries no error text (srv allow-lists what leaves a machine).
                "failed" if s(d, "error").is_empty() => out.push_str(" Its download failed."),
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
            // A remote answer carries no release URL either: the latest release's page is fixed.
            let url = Some(s(st, "url")).filter(|u| !u.is_empty()).unwrap_or("https://github.com/agentmuxai/agentmux/releases/latest");
            out.push_str(&format!(" This install can't upgrade itself yet; the user can download it from {url}."));
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
    fn disk_lists_volumes_build_folders_and_says_when_sizes_are_lower_bounds() {
        let gb = 1_073_741_824u64;
        let report = json!({
            "data_volume": { "free_bytes": 10 * gb, "total_bytes": 100 * gb },
            "agents_volume": null,
            "build_folders": [
                { "clone": "korp/agentmux", "target_bytes": 30 * gb, "incremental_bytes": 5 * gb, "complete": false },
                { "clone": "posa/agentmux", "target_bytes": 2 * gb, "incremental_bytes": 0, "complete": true }
            ],
            "build_folders_total_bytes": 32 * gb,
            "older_versions": [{ "version": "0.60.7", "bytes": 3 * gb, "complete": true }, { "version": "0.60.6", "bytes": gb, "complete": false }],
            "complete": false,
            "note": "Older version folders are removed by a person."
        });
        let text = describe_disk("area54", &report);
        assert!(text.contains("data volume: 10.0 GB free of 100.0 GB"), "{text}");
        assert!(!text.contains("agents volume"), "{text}");
        assert!(text.contains("korp/agentmux: 30.0 GB+ (incremental 5.0 GB)"), "{text}");
        assert!(text.contains("posa/agentmux: 2.0 GB (incremental 0.0 GB)"), "{text}");
        assert!(text.contains("0.60.7: 3.0 GB"), "{text}");
        assert!(text.contains("0.60.6: 1.0 GB+"), "{text}");
        assert!(text.contains("lower bounds"), "{text}");
    }

    #[test]
    fn history_reclaim_says_who_asked_when_it_runs_and_how_to_take_it_back() {
        let pending = json!({ "pending": true, "requested_by": "loap-3", "requested_at_ms": 1, "last_done_ms": 0, "last_before_bytes": 0, "last_after_bytes": 0 });
        let text = describe_reclaim(&pending, "request");
        assert!(text.contains("is asked for (by loap-3)"), "{text}");
        assert!(text.contains("next start of AgentMux") && text.contains("History > Storage"), "{text}");
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
    fn a_remote_status_without_a_url_or_an_error_still_reads_whole() {
        // What `app.version` gives: srv leaves the release URL and the download's error text out.
        let st = json!({ "state": "available", "current": "0.60.7", "latest": "0.60.8", "install": "windows-portable",
            "can_download": false, "checked_ms": 1 });
        let text = describe_status(&st);
        assert!(text.contains("download it from https://github.com/agentmuxai/agentmux/releases/latest."), "{text}");
        let failed = json!({ "state": "available", "current": "0.60.7", "latest": "0.60.8", "install": "windows-setup",
            "can_download": true, "download": { "version": "0.60.8", "state": "failed" } });
        let text = describe_status(&failed);
        assert!(text.contains("Its download failed.") && !text.contains("failed:"), "{text}");
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
