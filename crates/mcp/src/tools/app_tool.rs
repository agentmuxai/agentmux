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
            other => anyhow::bail!("App: unknown action {other:?}; use \"version\" or \"upgrade\""),
        },
        _ => Err(not_in_family()),
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
        "off" => out.push_str(" The update check is turned off."),
        _ => out.push_str(" Not checked for a newer release yet."),
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
        if st["can_download"].as_bool() == Some(true) {
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
        "pending_user" => format!(
            "Asked the user: every AgentMux window shows your request to upgrade to {version}. You'll get an \
             [AgentMux] note with their answer; don't ask again meanwhile."
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
        assert!(describe_outcome(&json!({ "status": "pending_user", "version": "0.59.27", "id": "r" })).contains("[AgentMux] note"));
        assert!(describe_outcome(&json!({ "status": "up_to_date", "current": "0.59.27" })).contains("latest release"));
        let u = describe_outcome(&json!({ "status": "unavailable", "message": "no", "url": "https://x" }));
        assert_eq!(u, "Can't upgrade: no. The release page: https://x");
    }
}
