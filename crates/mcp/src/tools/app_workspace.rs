// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `App` with `workspace` and `keep_running`: an agent's view of its workspace's pending stop, and its request to
//! put it back (srv's `GET /api/v1/app/workspace` and `POST /api/v1/agent/keep-running`). Both work for host and
//! container agents. The request is recorded as the agent; whether to honour it is up to the workspace's timer.

use super::app_tool::str_arg;
use super::*;

/// The most one request may ask for, minutes (srv refuses more).
const MAX_MINUTES: u64 = 240;

/// `workspace`: whether a stop is pending, when and why, and what an agent last asked for.
pub(super) async fn workspace(cx: &ToolCtx<'_>) -> Result<String> {
    let url = format!("{}/api/v1/app/workspace", cx.local_url.trim_end_matches('/'));
    let resp = cx.client.get(&url).header(AUTH_KEY_HEADER, cx.auth_key).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App workspace: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(describe(&body, agentmux_common::time::now_ms()))
}

/// `keep_running` with `minutes`: ask for the pending stop to be put back.
pub(super) async fn keep_running(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let Some(minutes) = arguments.get("minutes").and_then(Value::as_u64).filter(|m| (1..=MAX_MINUTES).contains(m)) else {
        anyhow::bail!("App keep_running: minutes is a whole number from 1 to {MAX_MINUTES}{}", str_arg(arguments, "minutes").map(|m| format!(", not {m:?}")).unwrap_or_default());
    };
    let url = format!("{}/api/v1/agent/keep-running", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&json!({ "minutes": minutes }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("App keep_running: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(format!(
        "Asked for the stop to be put back by {minutes} minutes, recorded as {}. {}",
        body["by"].as_str().unwrap_or("you"),
        body["note"].as_str().unwrap_or("The workspace's timer decides whether to honour it.")
    ))
}

/// The answer of `workspace` in words: "This workspace stops at 14:32 UTC (in 12 minutes) because nothing has
/// happened for 50 minutes."
pub(super) fn describe(body: &Value, now_ms: i64) -> String {
    let asked = body["keep_running"].as_object().map(|k| {
        format!(
            " {} asked for {} more minutes{}.",
            k.get("by").and_then(Value::as_str).unwrap_or("an agent"),
            k.get("minutes").and_then(Value::as_u64).unwrap_or(0),
            if body["keep_running"]["at_ms"].is_u64() { " (waiting for the timer to answer)" } else { "" }
        )
    });
    if body["stopping"] != Value::Bool(true) {
        return "No stop is pending for this workspace.".to_string();
    }
    let at = body["stop_at_ms"].as_i64().unwrap_or(0);
    let secs = at / 1000;
    let clock = format!("{:02}:{:02} UTC", (secs / 3600) % 24, (secs / 60) % 60);
    let left = ((at - now_ms) / 60_000).max(0);
    let why = body["reason"].as_str().unwrap_or("");
    let mut out = format!("This workspace stops at {clock} (in {left} minutes) because {why}.");
    if let Some(a) = asked {
        out.push_str(&a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_stop_says_so() {
        assert_eq!(describe(&json!({ "stopping": false, "keep_running": null }), 0), "No stop is pending for this workspace.");
    }

    #[test]
    fn a_stop_says_when_and_why_and_what_was_asked() {
        // 1_800_000_000 s is 08:00 UTC; the stop is at 08:12 and it is 08:00 now.
        let now = 1_800_000_000_000i64;
        let stop = json!({ "stopping": true, "stop_at_ms": now + 12 * 60_000, "reason_id": "timer", "reason": "nothing has happened for 50 minutes", "keep_running": null });
        assert_eq!(describe(&stop, now), "This workspace stops at 08:12 UTC (in 12 minutes) because nothing has happened for 50 minutes.");
        let asked = json!({ "stopping": true, "stop_at_ms": now + 12 * 60_000, "reason_id": "person", "reason": "you asked", "keep_running": { "minutes": 30, "by": "agent:agenta", "at_ms": now } });
        assert!(describe(&asked, now).ends_with("agent:agenta asked for 30 more minutes (waiting for the timer to answer)."));
    }

    #[test]
    fn a_stop_in_the_past_does_not_read_as_negative_minutes() {
        let now = 1_800_000_000_000i64;
        let late = json!({ "stopping": true, "stop_at_ms": now - 5 * 60_000, "reason": "your credit ran out" });
        assert!(describe(&late, now).contains("(in 0 minutes)"));
    }
}
