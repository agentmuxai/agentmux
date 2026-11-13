// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` calls that wait for the person return "waiting" after a short grace
//! (docs/specs/SPEC_WAITING_CALLS_RETURN_AND_FREE_THE_TURN_2026_11_10.md): what the agent reads then, and the
//! two actions that ask about or take down such a request, `request_status` and `cancel_request`.

use anyhow::Result;
use serde_json::Value;

use super::*;

/// What a "waiting" answer starts with: the follow-ups a call asked for (`until_*`, `snapshot`) are not run
/// for it, since nothing has happened yet.
pub(super) const WAITING_PREFIX: &str = "Waiting for the person";

/// What the agent reads when srv answers a call with `waiting`, or `None` for any other answer. The text tells
/// it what to do: say what it waits for, end its turn, not call again.
pub(super) fn waiting_text(body: &Value) -> Option<String> {
    let d = body.get("data").filter(|d| d.get("waiting").and_then(Value::as_bool) == Some(true))?;
    let id = d.get("request").and_then(Value::as_str).unwrap_or("?");
    let question = d.get("question").and_then(Value::as_str).unwrap_or("something");
    let pane = d.get("pane").and_then(Value::as_str).unwrap_or("?");
    let minutes = d.get("expires_in_secs").and_then(Value::as_u64).map(|s| (s + 59) / 60);
    let until = minutes.map_or(String::new(), |m| format!(" It stays open for {m} min."));
    Some(format!(
        "{WAITING_PREFIX}: {question} (request {id}, pane {pane:?}).{until} Nothing has been done yet. Say in one line what you are \
         waiting for and END YOUR TURN: the person can ask you questions while they decide, and you are told how it ends. Do not call again \
         and do not ask again; if you need to take the question back, Browser cancel_request with request {id}, or Browser request_status \
         if you are unsure whether it is still open."
    ))
}

/// `request_status` (`cancel: false`) and `cancel_request` (`cancel: true`).
pub(super) async fn request(arguments: &Value, cx: &ToolCtx<'_>, cancel: bool) -> Result<String> {
    let name = if cancel { "cancel_request" } else { "request_status" };
    let text_arg = |k: &str| arguments.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (request, pane) = (text_arg("request"), text_arg("pane"));
    if request.is_none() && (cancel || pane.is_none()) {
        anyhow::bail!("Browser {name} needs request (the id a waiting answer gave){}", if cancel { "" } else { " or pane" });
    }
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let req = agentmux_common::api_types::UiBrowserRequestRequest { auth: sign_ui_automation_auth()?, request, pane, cancel };
    let url = format!("{}/api/v1/ui/browser/request", local_url.trim_end_matches('/'));
    let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&req).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Browser {name} failed: HTTP {status} — {text}");
    }
    let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!("Browser {name}: bad response: {e}"))?;
    let d = body.get("data").unwrap_or(&Value::Null);
    Ok(if cancel {
        format!("Request {} is taken down: nothing will be done for it, and nobody is waiting for the person's answer any more.", d["request"].as_str().unwrap_or("?"))
    } else {
        status_text(d)
    })
}

/// What the agent reads for a request's status.
fn status_text(d: &Value) -> String {
    let id = d["request"].as_str().unwrap_or("?");
    let question = d["question"].as_str().unwrap_or("?");
    let ago = |k: &str| d[k].as_u64().map(|s| if s < 90 { format!("{s} s ago") } else { format!("{} min ago", s / 60) }).unwrap_or_default();
    let detail = d["detail"].as_str().filter(|s| !s.is_empty()).map(|s| format!(" {s}")).unwrap_or_default();
    match d["status"].as_str().unwrap_or("") {
        "open" => format!(
            "Request {id} ({question}) is still waiting for the person (asked {}). End your turn: you are told how it ends.",
            ago("started_secs_ago")
        ),
        "done" => format!("Request {id} ({question}): the person approved it and it was done ({}).{detail}", ago("ended_secs_ago")),
        "declined" => format!("Request {id} ({question}): the person declined it ({}). Don't retry it; ask them what to do.", ago("ended_secs_ago")),
        "timed_out" => format!("Request {id} ({question}): nobody answered in time ({}). Ask them in chat before trying again.", ago("ended_secs_ago")),
        "void" => format!("Request {id} ({question}): void, nothing was done ({}):{detail}", ago("ended_secs_ago")),
        "failed" => format!("Request {id} ({question}): the person approved it, but it wasn't done ({}):{detail}", ago("ended_secs_ago")),
        "cancelled" => format!("Request {id} ({question}): you took it back ({}).", ago("ended_secs_ago")),
        other => format!("Request {id}: {other}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_waiting_answer_tells_the_agent_to_end_its_turn_and_not_call_again() {
        let body = json!({ "ok": true, "data": { "waiting": true, "request": "r-1", "kind": "approval", "pane": "p1", "question": "Click \"Send\" on https://a.test/", "expires_in_secs": 598 } });
        let t = waiting_text(&body).unwrap();
        assert!(t.starts_with(WAITING_PREFIX));
        for want in ["request r-1", "Click \"Send\"", "END YOUR TURN", "Do not call again", "cancel_request", "Nothing has been done yet", "10 min"] {
            assert!(t.contains(want), "{want}: {t}");
        }
    }

    #[test]
    fn any_other_answer_is_not_a_waiting_one() {
        assert_eq!(waiting_text(&json!({ "ok": true, "data": { "after": null } })), None);
        assert_eq!(waiting_text(&json!({ "ok": true, "data": { "waiting": false } })), None);
        assert_eq!(waiting_text(&json!({ "ok": true })), None);
    }

    #[test]
    fn each_status_says_what_happened_and_what_to_do() {
        let d = |status: &str, detail: Option<&str>| {
            json!({ "request": "r-2", "question": "Click \"Pay\"", "status": status, "detail": detail, "started_secs_ago": 30, "ended_secs_ago": 200 })
        };
        assert!(status_text(&d("open", None)).contains("still waiting for the person (asked 30 s ago). End your turn"));
        assert!(status_text(&d("done", Some("The page is now https://a.test/paid."))).contains("approved it and it was done (3 min ago). The page is now"));
        assert!(status_text(&d("declined", None)).contains("Don't retry it"));
        assert!(status_text(&d("timed_out", None)).contains("Ask them in chat"));
        assert!(status_text(&d("void", Some("the page changed"))).contains("void, nothing was done (3 min ago): the page changed"));
        assert!(status_text(&d("failed", Some("the form changed"))).contains("wasn't done (3 min ago): the form changed"));
        assert!(status_text(&d("cancelled", None)).contains("you took it back"));
    }
}
