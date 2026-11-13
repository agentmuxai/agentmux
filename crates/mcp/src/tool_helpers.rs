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
    let url = format!("{}/agentmux/jobs/{}", local_url.trim_end_matches('/'), id);
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
pub(crate) async fn await_shutdown(client: &reqwest::Client, local_url: &str, auth_key: &str, request_id: &str, to_the_end: bool) -> Value {
    await_shutdown_of(client, local_url, auth_key, request_id, to_the_end, None).await
}

/// [`await_shutdown`] for a caller whose own pane may be one of several in a request: it also stops
/// waiting once the user has answered for `own` (kept it), which the request's shared status does
/// not say until every other agent has been dealt with.
pub(crate) async fn await_shutdown_of(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    request_id: &str,
    to_the_end: bool,
    own: Option<&str>,
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
        if own.is_some_and(|b| own_outcome(last.clone(), b)["status"] == "kept_by_user") {
            break;
        }
        match last["status"].as_str().unwrap_or("") {
            "" | "pending" => {}
            "proceeding" if to_the_end => {}
            _ => break,
        }
    }
    last
}

/// srv's name for a pending shutdown's tool (its audit log keeps the endpoint
/// names) as agents call it now.
pub(crate) fn public_stop(via: &str) -> &str {
    match via {
        "QuitSelf" => "Stop self",
        "ClosePane" => "Stop pane",
        "FleetBulkStop" => "Stop agents",
        other => other,
    }
}

/// What `ClosePane block_id=` tells the agent once the target's user has
/// answered (§6.5).
pub(crate) fn close_pane_outcome(block: &str, now: &Value) -> anyhow::Result<String> {
    // Joined a shutdown already under way that does less than close the pane. Browser close is a pane close too.
    if let Some(via) = now["via"].as_str().filter(|v| !matches!(*v, "ClosePane" | "Browser close")).map(public_stop) {
        let by = now["by"].as_str().unwrap_or("another agent");
        return match now["status"].as_str().unwrap_or("") {
            "kept_by_user" => Ok(format!("Not closed: the user chose to keep pane {block:?} running.")),
            // Found gone (closed, or its agent already stopped): retrying
            // would only find it gone again.
            "superseded" => Ok(format!("Pane {block:?} was already closed, or its agent already stopped: nothing more to do.")),
            "failed" => anyhow::bail!("close failed: {}", now["error"].as_str().unwrap_or("no detail")),
            "" | "pending" => anyhow::bail!("close of {block:?}: no answer from the user's override window"),
            state => Ok(format!(
                "{by}'s {via} was already shutting pane {block:?}'s agent down ({state}); the pane itself may still be open — check Pane layout, and close an agent with Stop action \"agents\" or any other pane with action \"pane\"."
            )),
        };
    }
    match now["status"].as_str().unwrap_or("") {
        "shut_down" => Ok(format!("Closed pane {block:?}: its user didn't keep it in time.")),
        "kept_by_user" => Ok(format!("Not closed: the user chose to keep pane {block:?} running.")),
        "superseded" => Ok(format!("Pane {block:?} was already closed.")),
        "proceeding" => Ok(format!("Pane {block:?} is shutting down: its user didn't keep it.")),
        "failed" => anyhow::bail!("close failed: {}", now["error"].as_str().unwrap_or("no detail")),
        _ => anyhow::bail!("close of {block:?}: no answer from the user's override window"),
    }
}

/// One target's own answer from a settled request over several agents: srv lists what became of each
/// agent (`outcomes`), while the request's own `status` is for all of them. A request for one agent
/// has no list, and its answer is the target's own.
pub(crate) fn own_outcome(now: Value, block: &str) -> Value {
    let Some(o) = now["outcomes"].as_array().and_then(|all| all.iter().find(|o| o["block_id"].as_str() == Some(block))) else {
        return now;
    };
    let mut own = now.clone();
    own["status"] = o["outcome"].clone();
    own["error"] = o.get("error").cloned().unwrap_or(Value::Null);
    own
}

/// What `Stop pane` and `Stop agents` say instead of waiting for the user's window: the call returns
/// at once, srv runs the window, and the agent is told by one `[AgentMux]` note only if the user keeps
/// what it asked to close or the close fails.
pub(crate) fn pending_note(secs: u64) -> String {
    format!("Not closed yet: the user has {secs} s to keep it running, and it closes after that unless they do. Don't wait or poll: you get one [AgentMux] note only if they keep it or the close fails.")
}

/// The windows a stop opened, one per request id (several targets can share one): their targets and
/// when each closes unless the user keeps it. `pending` is srv's list of {block_id, request_id,
/// deadline_ms?}; `fallback_deadline_ms` is the answer's own deadline, for an entry without one.
pub(crate) fn pending_windows(pending: &[Value], fallback_deadline_ms: Option<i64>, now_ms: i64) -> Vec<Value> {
    let mut windows: Vec<(String, Vec<String>, Option<i64>)> = Vec::new();
    for p in pending {
        let id = p["request_id"].as_str().unwrap_or_default().to_string();
        let block = p["block_id"].as_str().unwrap_or_default().to_string();
        let deadline = p["deadline_ms"].as_i64().or(fallback_deadline_ms);
        match windows.iter_mut().find(|w| w.0 == id) {
            Some(w) => w.1.push(block),
            None => windows.push((id, vec![block], deadline)),
        }
    }
    windows
        .into_iter()
        .map(|(id, targets, deadline)| {
            let mut w = json!({ "id": id, "targets": targets });
            if let Some(at) = deadline {
                w["closes_at"] = json!(at);
                w["closes_in_s"] = json!(((at - now_ms).max(0) + 999) / 1000);
            }
            w
        })
        .collect()
}

/// `Stop pane` on another pane: srv's 202 answer, as the agent sees it. A request that joined a
/// shutdown already under way (`via` is not a pane close) says whose it is and that the pane may
/// outlive it.
pub(crate) fn close_pane_pending(block: &str, body: &Value, now_ms: i64) -> String {
    let entry = json!({ "block_id": block, "request_id": body["request_id"], "deadline_ms": body["deadline_ms"] });
    // A browser pane's window is shorter than the 30 s of the rest: say what srv set.
    let secs = body["deadline_ms"].as_i64().map_or(30, |d| ((d - now_ms).max(0) as u64).div_ceil(1000));
    let mut note = pending_note(secs);
    if let Some(via) = body["via"].as_str().filter(|v| !matches!(*v, "ClosePane" | "Browser close")).map(public_stop) {
        let by = body["by"].as_str().unwrap_or("another agent");
        note.push_str(&format!(" It joined {by}'s {via}, already under way, which may leave the pane itself open: check Pane layout."));
    }
    let out = json!({ "status": "pending", "pending": pending_windows(&[entry], None, now_ms), "note": note });
    serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string())
}

/// The `note` of a `Stop agents` result: what srv said (a passed `signal` or
/// `staged` no longer applies) and, for a target on an older AgentMux on this
/// machine, which only stops (its answer names the `effect` `stop`), that its
/// pane was left open.
pub(crate) fn add_stop_notes(result: &mut Value, srv_note: Option<String>, pending: &[Value]) {
    let older: Vec<&str> = pending
        .iter()
        .filter(|p| p["effect"].as_str() == Some("stop"))
        .filter_map(|p| p["block_id"].as_str())
        .collect();
    let mut notes: Vec<String> = srv_note.into_iter().collect();
    if !older.is_empty() {
        notes.push(format!(
            "{} run on an older AgentMux on this machine: stopped, but the pane was left open; the user has to close it",
            older.join(", ")
        ));
    }
    if !notes.is_empty() {
        result["note"] = json!(notes.join("; "));
    }
}

/// What `QuitSelf` tells the agent once the user's override window has
/// settled (§6.5).
pub(crate) fn quit_self_outcome(state: &str, body: &Value) -> anyhow::Result<String> {
    match state {
        "kept_by_user" => Ok("The user chose to keep you running. Do NOT quit and do not call Stop self again for this; carry on.".to_string()),
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

/// `held_reason` of a message kept for a session gone cold (srv's
/// `cold_hold::REASON_IDLE`).
pub(crate) const HELD_IDLE: &str = "idle";

/// `held_reason` of a routine message kept back by quota mode (srv's `cold_hold::REASON_QUOTA`).
pub(crate) const HELD_QUOTA: &str = "quota";

/// Which of the two "kept until the agent is next active" holds srv's answer names, for a send to
/// many: a held message is not a failed one.
pub(crate) fn held_until_active(result: &Value) -> Option<&'static str> {
    match result.get("held_reason").and_then(|v| v.as_str()) {
        Some(HELD_IDLE) => Some(HELD_IDLE),
        Some(HELD_QUOTA) => Some(HELD_QUOTA),
        _ => None,
    }
}

/// How long a send waits for srv's answer. srv's own tiers for an agent it does not run (a LAN peer, the cloud
/// relay, then the hold) can add up to more than the client's default 10 s, and srv finishes the delivery anyway.
pub(crate) const INJECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// What a send tells the sender when srv's answer did not arrive, in place of the transport's own words
/// ("error sending request for url ..."), which say nothing about whether the message was kept.
pub(crate) fn inject_transport_error(e: &reqwest::Error) -> String {
    if e.is_connect() {
        format!("srv could not be reached, so nothing was sent ({e})")
    } else if e.is_timeout() {
        format!(
            "srv gave no answer within {} s. It carries on with the message (delivers it or holds it for the agent), so look at Agents conversations before sending it again",
            INJECT_TIMEOUT.as_secs()
        )
    } else {
        format!("the answer from srv was lost ({e}). The message may have been delivered or held: look at Agents conversations before sending it again")
    }
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
    // srv refused it before delivery (a reply that says nothing): nothing was sent, no turn started.
    if result.get("refused").and_then(|v| v.as_bool()) == Some(true) {
        let why = result.get("error").and_then(|v| v.as_str()).unwrap_or("Not sent.");
        return Ok(format!("NOT SENT to {to} — {why}"));
    }
    if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
        return sent_text(to, result, &id);
    }
    if result.get("held").and_then(|v| v.as_bool()) == Some(true) {
        return held_text(to, result, &id);
    }
    delivery_failure(to, result)
}

/// The reply to `success: true`, which is either a delivery or a message queued at the relay.
fn sent_text(to: &str, result: &Value, id: &str) -> Result<String> {
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
    // What srv could see when it queued the message: the target runs on no LAN peer (or it would
    // have been delivered there), so only an install of the sender's elsewhere can pick it up.
    let nowhere_near = match result.get("lan_peers").and_then(|v| v.as_array()) {
        Some(peers) => {
            let seen: Vec<&str> = peers.iter().filter_map(|p| p.as_str()).collect();
            let peers_said = if seen.is_empty() { "no LAN peers found".to_string() } else { format!("LAN peers: {}", seen.join("; ")) };
            format!(
                " {to} is not running on this machine or on any LAN peer ({peers_said}), so it \
                 arrives only if an install of yours elsewhere runs {to} and is online."
            )
        }
        None => String::new(),
    };
    Ok(format!(
        "QUEUED for {to} via the cloud relay (unconfirmed, expires in 30 min) — NOT yet \
         delivered. The relay accepted it; their AgentMux picks it up when it next syncs, \
         and the relay drops it after 30 minutes if nothing does (which is what happens if \
         that instance stays offline). If {to} has not written to you by then, you get a note \
         that it was never confirmed. You get this same result for an agent name that \
         does not exist anywhere, so check the spelling against Agents list if you \
         expected local delivery.{nowhere_near}{not_yours}{id}"
    ))
}

/// The reply to `held: true`: srv kept the message and says why and until when.
fn held_text(to: &str, result: &Value, id: &str) -> Result<String> {
    // SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md: the target is a known
    // agent that is not running anywhere srv can reach, so srv kept the
    // message and delivers it when the agent starts. Since Phase 0 item 4
    // it also holds for a receiver that is here but not signed in.
    // SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §5: the agent is here but
    // its account is out of quota.
    if result.get("held_reason").and_then(|v| v.as_str()) == Some("usage_limit") {
        let until = match result.get("held_until").and_then(|v| v.as_str()) {
            Some(t) => format!(", until {t}"),
            None => String::new(),
        };
        return Ok(format!(
            "HELD for {to} (usage_limit{until}) — not delivered yet. {to}'s account is out of \
             quota; this AgentMux instance (channel) keeps the message and delivers it when \
             {to} can work again (an account move or the reset), for up to 24 hours. Do not \
             resend it.{id}"
        ));
    }
    if result.get("held_reason").and_then(|v| v.as_str()) == Some(HELD_IDLE) {
        return Ok(format!(
            "HELD for {to} (idle) — not delivered yet. {to}'s session has been idle for about an \
             hour, so waking it costs a full restart of its context; this AgentMux instance \
             (channel) keeps the message and delivers it when {to} is next active, for up to 24 \
             hours. If it is work for {to} or cannot wait, pass urgent: true (send it again) \
             to wake it now; the held copy is dropped. Otherwise do not resend it.{id}"
        ));
    }
    if result.get("held_reason").and_then(|v| v.as_str()) == Some(HELD_QUOTA) {
        return Ok(format!(
            "HELD for {to} (quota) — not delivered yet. Quota mode is tight and {to} has no turn in \
             flight, so this routine message would start one; this AgentMux instance (channel) keeps \
             it and delivers it when {to} is next active or the mode is off, for up to 24 hours. If it \
             is work for {to} or cannot wait, pass urgent: true (send it again) to deliver it now; the \
             held copy is dropped. Otherwise do not resend it.{id}"
        ));
    }
    if result.get("held_reason").and_then(|v| v.as_str()) == Some("needs_login") {
        return Ok(format!(
            "HELD for {to} (needs_login) — not delivered yet. {to}'s agent needs to sign in \
             again; this AgentMux instance (channel) keeps the message \
             and delivers it within about a minute of their signing in, for up to 24 hours. \
             Do not resend it.{id}"
        ));
    }
    if result.get("held_reason").and_then(|v| v.as_str()) == Some("peer_offline") {
        let host = result.get("held_host").and_then(|v| v.as_str()).map_or_else(|| "its machine".to_string(), str::to_string);
        return Ok(format!(
            "HELD for {to} (peer_offline) — not delivered yet. {to} runs on {host}, which is not \
             answering on the LAN; this AgentMux instance (channel) keeps the message and \
             delivers it when {host} is seen again, for up to 24 hours (the cloud relay would drop \
             it after 30 minutes). Do not resend it.{id}"
        ));
    }
    if let Some(why) = result.get("start_blocked").and_then(|v| v.as_str()) {
        return Ok(format!(
            "HELD for {to} (not_running, capacity) — not delivered yet. {to} was stopped and AgentMux \
             did not start it: {why}. It keeps the message for up to 24 hours and delivers it when {to} \
             starts; the start is tried again when the next message for {to} arrives, or start it with \
             Agents start once there is room. Do not resend it.{id}"
        ));
    }
    if result.get("starting").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(format!(
            "HELD for {to} (not_running, starting) — not delivered yet. {to} was stopped; \
             AgentMux is starting it now and delivers this message as soon as it is up, \
             normally within a few seconds (kept for up to 24 hours if the start fails). Do \
             not resend it.{id}"
        ));
    }
    Ok(format!(
        "HELD for {to} (not_running) — not delivered yet. {to} is not running; this \
         AgentMux instance (channel) keeps the message and delivers it when {to} starts \
         here, for up to 24 hours. Do not resend it.{id}"
    ))
}

/// The error for a message srv neither delivered nor held.
fn delivery_failure(to: &str, result: &Value) -> Result<String> {
    let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
    // Two failures the sender can act on differently. A spawn-gate refusal is
    // recoverable (the receiver signs in). srv now holds it (`needs_login`
    // above); this answer remains for an older srv, a cron sender or a
    // non-host tier, which are not held.
    if err.starts_with("identity spawn gate")
        || (result.get("refused_reason").and_then(|v| v.as_str()) == Some("needs_login") && !err.starts_with("agent unavailable: "))
    {
        anyhow::bail!(
            "Message delivery failed (needs_login): {to}'s agent cannot start because it is not \
             signed in, and the message was NOT kept. Send it again after they sign in. ({err})"
        )
    }
    if err.starts_with("agent not found") {
        anyhow::bail!("Message delivery failed (not_found): {err}")
    }
    // An agent out of quota or signed out, where it can't be held (a LAN or
    // cross-channel hop): srv's `UNAVAILABLE_PREFIX`.
    if let Some(why) = err.strip_prefix("agent unavailable: ") {
        // What ends the wait is data in the answer (`retry_when`); a srv from before it
        // is read from the wording.
        let signs_in = match result.get("retry_when").and_then(|v| v.as_str()) {
            Some(when) => when == "sign_in",
            None => why.ends_with(" needs to sign in again"),
        };
        let retry = if signs_in {
            "after they sign in".to_string()
        } else {
            format!("when {to} can work (an account move or the reset)")
        };
        anyhow::bail!(
            "Message delivery failed (unavailable): {why}, and the message was NOT kept. Send it \
             again {retry}."
        )
    }
    anyhow::bail!("Message delivery failed: {err}")
}
