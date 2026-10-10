// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Helpers the tool handlers share: cron, durations, quit-self and bulk-stop outcomes, delivery text.
//! Split out of main.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2).

use super::*;

pub(crate) async fn cron_set_enabled(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    id: &str,
    action: &str,
) -> Result<String> {
    let url = format!("{}/agentmux/cron/{}", local_url.trim_end_matches('/'), id);
    let body = serde_json::json!({"action": action});
    let resp = client.patch(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
        .map_err(|e| anyhow::anyhow!("cron {action} request failed: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(format!("Job '{id}' not found"));
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Cron{} failed: HTTP {status} — {text}", if action == "pause" { "Pause" } else { "Resume" });
    }
    Ok(format!("Cron job {id} {}", if action == "pause" { "paused" } else { "resumed" }))
}

/// Parse a loop interval string into a `Duration`. Accepts a number with an
/// optional unit suffix: `s` (seconds), `m` (minutes), `h` (hours); a bare
/// number is treated as minutes. Clamped to [10s, 24h].
pub(crate) fn parse_interval(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("interval is empty");
    }
    let (num_part, mult_secs) = if let Some(n) = s.strip_suffix('s').or_else(|| s.strip_suffix('S')) {
        (n, 1.0_f64)
    } else if let Some(n) = s.strip_suffix('m').or_else(|| s.strip_suffix('M')) {
        (n, 60.0)
    } else if let Some(n) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        (n, 3600.0)
    } else {
        (s, 60.0) // bare number → minutes
    };
    let val: f64 = num_part
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid interval '{s}' (expected e.g. 30s, 5m, 1h)"))?;
    if !val.is_finite() || val <= 0.0 {
        anyhow::bail!("interval must be a positive number: '{s}'");
    }
    let secs = (val * mult_secs).round() as u64;
    Ok(Duration::from_secs(secs.clamp(10, 24 * 3600)))
}

/// Human-readable representation of a clamped Duration — used in the Loop
/// success message so the reported cadence matches the actual run cadence.
pub(crate) fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s % 3600 == 0 {
        format!("{}h", s / 3600)
    } else if s % 60 == 0 {
        format!("{}m", s / 60)
    } else {
        format!("{}s", s)
    }
}

/// What `QuitSelf` tells the agent, from srv's answer (§4.2, §6.3).
pub(crate) fn quit_self_result(status: u16, body: &Value) -> anyhow::Result<String> {
    match status {
        202 => Ok("Quit scheduled. Your session ends when this turn finishes: give the user a one-line goodbye and start no new work.".to_string()),
        200 => Ok("Already quitting — nothing more to do.".to_string()),
        403 => anyhow::bail!(
            "QuitSelf refused ({}): only the user's own message that started this turn can ask you to quit, and nothing else may have been delivered since. Do NOT retry. Tell the user they can type /quit in your pane.",
            body.get("refused").and_then(|v| v.as_str()).unwrap_or("not allowed")
        ),
        _ => anyhow::bail!("QuitSelf failed: HTTP {status} — {}", body),
    }
}

/// Polls srv for a shutdown waiting on the user's override
/// (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5) until it leaves `pending` — and
/// `proceeding` too when `to_the_end` — and returns the last status seen. One
/// long request instead would outlast the client's own timeout.
pub(crate) async fn await_shutdown(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    request_id: &str,
    to_the_end: bool,
) -> Value {
    let url = format!("{}/api/v1/agent/shutdown/{request_id}", local_url.trim_end_matches('/'));
    // The window, the turn end a self-quit waits for (30 s) and the graceful
    // stop itself, with room to spare.
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let mut last = Value::Null;
    while std::time::Instant::now() < give_up {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Ok(r) = client.get(&url).header(AUTH_KEY_HEADER, auth_key).send().await {
            last = r.json().await.unwrap_or(last);
        }
        match last["status"].as_str().unwrap_or("") {
            "" | "pending" => {}
            "proceeding" if to_the_end => {}
            _ => break,
        }
    }
    last
}

/// What `ClosePane block_id=` tells the agent once the target's user has
/// answered (§6.5).
pub(crate) fn close_pane_outcome(block: &str, now: &Value) -> anyhow::Result<String> {
    // Joined a shutdown already under way that does less than close the pane.
    if let Some(via) = now["via"].as_str().filter(|v| *v != "ClosePane") {
        let by = now["by"].as_str().unwrap_or("another agent");
        return match now["status"].as_str().unwrap_or("") {
            "kept_by_user" => Ok(format!("Not closed: the user chose to keep pane {block:?} running.")),
            // Found gone (closed, or its agent already stopped): retrying
            // would only find it gone again.
            "superseded" => Ok(format!("Pane {block:?} was already closed, or its agent already stopped: nothing more to do.")),
            "failed" => anyhow::bail!("close failed: {}", now["error"].as_str().unwrap_or("no detail")),
            "" | "pending" => anyhow::bail!("close of {block:?}: no answer from the user's override window"),
            state => Ok(format!(
                "{by}'s {via} was already shutting pane {block:?}'s agent down ({state}); the pane itself may still be open — call ClosePane again to close it."
            )),
        };
    }
    match now["status"].as_str().unwrap_or("") {
        "shut_down" => Ok(format!("Closed pane {block:?}: its user didn't keep it within 15 s.")),
        "kept_by_user" => Ok(format!("Not closed: the user chose to keep pane {block:?} running.")),
        "superseded" => Ok(format!("Pane {block:?} was already closed.")),
        "proceeding" => Ok(format!("Pane {block:?} is shutting down: its user didn't keep it.")),
        "failed" => anyhow::bail!("close failed: {}", now["error"].as_str().unwrap_or("no detail")),
        _ => anyhow::bail!("close of {block:?}: no answer from the user's override window"),
    }
}

/// Folds each waited-on target into `FleetBulkStop`'s usual shape: stopped
/// (or already gone) → `succeeded`; kept by the user or failed → `failed`,
/// with the kept ones also listed in `kept_by_user`.
pub(crate) fn merge_bulk_stop_outcomes(pending: Value, outcomes: Vec<(String, Value)>) -> Value {
    let mut succeeded = pending["succeeded"].as_array().cloned().unwrap_or_default();
    let mut failed = pending["failed"].as_array().cloned().unwrap_or_default();
    let mut kept = Vec::new();
    for (block, now) in outcomes {
        match now["status"].as_str().unwrap_or("") {
            "shut_down" | "superseded" | "proceeding" => succeeded.push(json!(block)),
            "kept_by_user" => {
                failed.push(json!({ "id": block, "error": "the user kept it running" }));
                kept.push(json!(block));
            }
            "failed" => failed.push(json!({ "id": block, "error": now["error"].as_str().unwrap_or("stop failed") })),
            _ => failed.push(json!({ "id": block, "error": "no answer from the user's override window" })),
        }
    }
    json!({
        "succeeded": succeeded,
        "failed": failed,
        "kept_by_user": kept,
        "aborted_early": pending["aborted_early"].as_bool().unwrap_or(false),
    })
}

/// What `QuitSelf` tells the agent once the user's override window has
/// settled (§6.5).
pub(crate) fn quit_self_outcome(state: &str, body: &Value) -> anyhow::Result<String> {
    match state {
        "kept_by_user" => Ok("The user chose to keep you running. Do NOT quit and do not call QuitSelf again for this; carry on.".to_string()),
        "proceeding" | "shut_down" => Ok("The user didn't stop it: you shut down when this turn ends. Give the user a one-line goodbye and start no new work.".to_string()),
        "superseded" => Ok("The user closed you themselves meanwhile.".to_string()),
        _ => anyhow::bail!(
            "QuitSelf: the shutdown {state}: {}",
            body.get("error").and_then(|v| v.as_str()).unwrap_or("no detail")
        ),
    }
}

/// `SendMessage`'s answer when srv accepted the message but queued it because
/// the target's process is starting up, restarting or stopping
/// (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.1). A target that is up gets
/// it at once, mid-turn included, and is reported as delivered.
pub(crate) fn deferred_delivery_text(to: &str) -> String {
    format!(
        "QUEUED for {to} — their agent is starting up, restarting or stopping, \
         so it has not reached them yet. Their AgentMux delivers it as soon as \
         their agent is up. Don't resend it."
    )
}

/// What `SendMessage` tells the sender, from srv's `/agentmux/reactive/inject`
/// body. Each answer keeps its first word (`Delivered`, `QUEUED`, `HELD`) so
/// existing prompts keep working, and adds the message id and the receiver's
/// condition where srv's answer already implies it
/// (SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md §5.2, Phase 0 item 3).
pub(crate) fn send_message_outcome(to: &str, result: &Value) -> Result<String> {
    // srv echoes the sender's own msgid, or the cloud relay's injection id on
    // the relay branch; it is the id the receiver's `MSGID=` shows.
    let id = match result.get("request_id").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => format!(" id={id}"),
        _ => String::new(),
    };
    if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
        // `success: true` spans two very different outcomes and the old
        // message conflated them, which made agent-to-agent delivery
        // unfalsifiable from the sender side: a name that exists on no
        // machine anywhere reported exactly the same string as a message
        // injected into a live conversation. Verified against a running
        // srv — `SendMessage(to="definitely-not-a-real-agent-xyz123")`
        // returned "Message sent to ...", and the server log showed "cloud
        // relay: queued for WAN delivery" for it, identical to a real remote
        // agent.
        //
        // The response already carries the distinction: the handler sets
        // `block_id` to the receiving block on local/host delivery
        // (backend/reactive/handler.rs), while the cloud-relay path leaves it
        // `None` because no receiver has seen the message yet
        // (server/reactive.rs, `try_cloud_relay` — "Queued is not delivered").
        if result.get("block_id").and_then(|v| v.as_str()).is_some() {
            // srv queues a message while the target's process is starting
            // up, restarting or stopping (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md
            // §2.1) and says so with `deferred`; "injected" would be untrue.
            if result.get("deferred").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(format!("{}{id}", deferred_delivery_text(to)));
            }
            return Ok(format!("Delivered to {to} — injected into their running conversation.{id}"));
        }
        // The relay accepts any name; it says when the target is not one of
        // the sender's own account's agents. An agent
        // counts once it has run while signed in, so one of the sender's own
        // that hasn't yet still gets `false` and can still receive it (Codex
        // P2 on #4249): a hint, never "only another account".
        let not_yours = if result.get("target_in_account").and_then(|v| v.as_bool()) == Some(false) {
            " No agent of that name has signed in from your account yet, so check the spelling. \
             It is still delivered if one of your agents with that name signs in within 30 \
             minutes, or if another account has an agent with that name."
        } else {
            ""
        };
        return Ok(format!(
            "QUEUED for {to} via the cloud relay (unconfirmed, expires in 30 min) — NOT yet \
             delivered. The relay accepted it; their AgentMux picks it up when it next syncs, \
             and the relay drops it after 30 minutes if nothing does (which is what happens if \
             that instance stays offline). You get this same result for an agent name that \
             does not exist anywhere, so check the spelling against DiscoverAgents if you \
             expected local delivery.{not_yours}{id}"
        ));
    }
    if result.get("held").and_then(|v| v.as_bool()) == Some(true) {
        // SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md: the target is a known
        // agent that is not running anywhere srv can reach, so srv kept the
        // message and delivers it when the agent starts. Since Phase 0 item 4
        // it also holds for a receiver that is here but not signed in.
        if result.get("held_reason").and_then(|v| v.as_str()) == Some("needs_login") {
            return Ok(format!(
                "HELD for {to} (needs_login) — not delivered yet. {to}'s agent cannot start \
                 because it is not signed in; this AgentMux instance (channel) keeps the message \
                 and delivers it within about a minute of their signing in, for up to 24 hours. \
                 Do not resend it.{id}"
            ));
        }
        return Ok(format!(
            "HELD for {to} (not_running) — not delivered yet. {to} is not running; this \
             AgentMux instance (channel) keeps the message and delivers it when {to} starts \
             here, for up to 24 hours. Do not resend it.{id}"
        ));
    }
    let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
    // Two failures the sender can act on differently. A spawn-gate refusal is
    // recoverable (the receiver signs in). srv now holds it (`needs_login`
    // above); this answer remains for an older srv, a cron sender or a
    // non-host tier, which are not held.
    if err.starts_with("identity spawn gate") {
        anyhow::bail!(
            "Message delivery failed (needs_login): {to}'s agent cannot start because it is not \
             signed in, and the message was NOT kept. Send it again after they sign in. ({err})"
        )
    }
    if err.starts_with("agent not found") {
        anyhow::bail!("Message delivery failed (not_found): {err}")
    }
    anyhow::bail!("Message delivery failed: {err}")
}
