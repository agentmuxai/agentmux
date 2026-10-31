// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `host` argument of the MuxAPI tools: run an operation on another machine
//! (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.8).
//!
//! This machine's srv builds the request, signs it with the agent's own key and sends it
//! (`POST /api/v1/agent/remote/op`); the other machine's srv carries it out. The host must be
//! named on each call, never inherited from an earlier one, so a local action can't silently
//! become a remote one, and the answer says which machine gave it.

use super::*;

/// The `host` argument: a machine's name as `Agents list` shows it. Absent or blank means this machine.
pub(super) fn host_arg(arguments: &Value) -> Option<String> {
    arguments.get("host").and_then(Value::as_str).map(str::trim).filter(|h| !h.is_empty()).map(str::to_string)
}

/// Run `op` with `params` on `host` as this agent. The answer is srv's `RemoteOpResult` as JSON
/// (`status`, `result`, `audit_id`, `refused_reason`); an error is only a failure to ask.
pub(crate) async fn remote_op_call(cx: &ToolCtx<'_>, host: &str, op: &str, params: Value, reason: &str) -> Result<Value> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let url = format!("{}/api/v1/agent/remote/op", cx.local_url.trim_end_matches('/'));
    let body = json!({ "agent_id": agent_slug()?, "host": host, "op": op, "params": params, "reason": reason });
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let answer: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("{op} on {host}: srv answered {status}: {}", answer["error"].as_str().unwrap_or(&answer.to_string()));
    }
    Ok(answer)
}

/// What a non-`done` answer says, for the agent to read; `None` when the operation ran.
pub(crate) fn explain_not_done(host: &str, op: &str, answer: &Value) -> Option<String> {
    match answer["status"].as_str() {
        Some("done") => None,
        Some("queued") | Some("pending_veto") => None,
        Some("refused") => {
            let why = answer["refused_reason"].as_str().unwrap_or("no reason given");
            let message = answer["result"]["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default();
            Some(format!("{host} refused {op} ({why}){message}. {}", refusal_hint(why)))
        }
        _ => Some(format!("{op} on {host} failed: {}", answer["result"]["error"].as_str().unwrap_or("no reason given"))),
    }
}

/// What to do about a refusal the transport itself made.
fn refusal_hint(reason: &str) -> &'static str {
    match reason {
        "remote_ops_off" => "That machine has turned remote operations off (network:remote_ops in its settings).",
        "unknown_sender" | "unverified_sender" => "That machine does not yet trust this agent's key: ask one of its agents to jekt you once, so each side pins the other, then retry.",
        "bad_signature" | "wrong_audience" | "stale_request" => "Check that both machines' clocks are right and both run a build with remote operations.",
        "needs_certificate" => "That machine holds no certificate from your agent's install. Approve this install on that machine (the instance is listed there), then retry; a first-seen peer is not enough for account data or changes.",
        "install_not_trusted" => "That machine has not approved the install your agent runs on. Approve this instance on that machine, then retry.",
        "unknown_op" => "That machine's AgentMux does not know this operation: it needs a newer version.",
        "peer_cannot_do_remote_ops" => "That machine's AgentMux is too old for remote operations.",
        _ => "",
    }
}

/// `answer` as text, headed by the machine that gave it.
pub(crate) fn pretty(host: &str, answer: &Value) -> String {
    let body = serde_json::to_string_pretty(&answer["result"]).unwrap_or_else(|_| answer["result"].to_string());
    let status = answer["status"].as_str().unwrap_or("done");
    let mark = if status == "done" { String::new() } else { format!(" [{status}]") };
    format!("From {host}{mark}:\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_host_is_this_machine() {
        assert_eq!(host_arg(&json!({})), None);
        assert_eq!(host_arg(&json!({ "host": "  " })), None);
        assert_eq!(host_arg(&json!({ "host": " narko " })), Some("narko".to_string()));
    }

    #[test]
    fn a_refusal_says_who_refused_why_and_what_to_do() {
        let answer = json!({ "status": "refused", "refused_reason": "remote_ops_off", "result": null });
        let text = explain_not_done("narko", "app.version", &answer).unwrap();
        assert!(text.contains("narko refused app.version (remote_ops_off)") && text.contains("network:remote_ops"), "{text}");
        let none = json!({ "status": "refused", "result": null });
        assert!(explain_not_done("narko", "app.version", &none).unwrap().contains("no reason given"));
    }

    #[test]
    fn a_done_or_waiting_answer_is_not_an_error() {
        for status in ["done", "queued", "pending_veto"] {
            assert!(explain_not_done("narko", "x", &json!({ "status": status })).is_none(), "{status}");
        }
        assert!(explain_not_done("narko", "x", &json!({ "status": "failed", "result": { "error": "boom" } })).unwrap().contains("boom"));
    }

    #[test]
    fn the_answer_names_the_machine() {
        let text = pretty("narko", &json!({ "status": "done", "result": { "version": "0.60.7" } }));
        assert!(text.starts_with("From narko:") && text.contains("0.60.7"), "{text}");
        assert!(pretty("narko", &json!({ "status": "queued", "result": {} })).starts_with("From narko [queued]:"));
    }
}
