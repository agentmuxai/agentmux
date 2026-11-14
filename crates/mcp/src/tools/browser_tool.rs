// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser: one tool for the browser panes an agent drives, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S2). It
//! replaces OpenBrowser and the sixteen Browser* tools; their handlers
//! (browser.rs) run each action unchanged. An action that changes the page can
//! wait and read it again in the same call.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "open", tool: "OpenBrowser", rename: &[] },
    Act { action: "snapshot", tool: "BrowserSnapshot", rename: &[] },
    Act { action: "find", tool: "BrowserFind", rename: &[] },
    Act { action: "click", tool: "BrowserClick", rename: &[] },
    Act { action: "fill", tool: "BrowserFill", rename: &[] },
    Act { action: "request_value", tool: "BrowserRequestValue", rename: &[] },
    Act { action: "capture", tool: "BrowserCapture", rename: &[] },
    Act { action: "signin", tool: "BrowserSignin", rename: &[] },
    Act { action: "generate_password", tool: "BrowserGeneratePassword", rename: &[] },
    Act { action: "select", tool: "BrowserSelect", rename: &[] },
    Act { action: "check", tool: "BrowserCheck", rename: &[] },
    Act { action: "set_files", tool: "BrowserSetFiles", rename: &[] },
    Act { action: "navigate", tool: "BrowserNavigate", rename: &[] },
    Act { action: "back", tool: "BrowserBack", rename: &[] },
    Act { action: "forward", tool: "BrowserForward", rename: &[] },
    Act { action: "reload", tool: "BrowserReload", rename: &[] },
    Act { action: "zoom", tool: "BrowserZoom", rename: &[] },
    Act { action: "key", tool: "BrowserDispatchKey", rename: &[] },
    Act { action: "terminal", tool: "BrowserTerminal", rename: &[] },
    Act { action: "focus", tool: "BrowserFocusElement", rename: &[] },
    Act { action: "focus_info", tool: "BrowserFocusInfo", rename: &[] },
    Act {
        action: "wait",
        tool: "BrowserWaitFor",
        rename: &[("until_text", "text"), ("until_url", "url_contains"), ("until_gone", "gone"), ("until_state", "state")],
    },
    Act { action: "eval", tool: "BrowserEval", rename: &[] },
    Act { action: "handoff", tool: "BrowserHandoff", rename: &[] },
    Act { action: "handoff_status", tool: "BrowserHandoffStatus", rename: &[] },
    Act { action: "request_status", tool: "BrowserRequestStatus", rename: &[] },
    Act { action: "cancel_request", tool: "BrowserCancelRequest", rename: &[] },
    Act { action: "retake", tool: "BrowserRetake", rename: &[] },
    Act { action: "take_control", tool: "BrowserTakeControl", rename: &[] },
    Act { action: "release", tool: "BrowserRelease", rename: &[] },
    Act { action: "request_auto_approve", tool: "BrowserRequestBypass", rename: &[] },
    Act { action: "stop_auto_approve", tool: "BrowserEndBypass", rename: &[] },
    Act { action: "log", tool: "BrowserLog", rename: &[] },
    Act { action: "close", tool: "BrowserClose", rename: &[] },
];

/// Actions after which `until_*` and `snapshot` may follow in the same call.
const CHANGES_THE_PAGE: &[&str] = &["click", "fill", "select", "check", "key", "navigate", "back", "forward", "reload"];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Browser" => masked(relabeled("Browser", ACTS, run(arguments, cx).await)),
        _ => Err(not_in_family()),
    }
}

/// Everything the agent reads from a Browser call, with card, ID and bank numbers and one-time
/// codes hidden by their shape: the last line of the secret-field rule (the host hides a secret
/// *field*'s value; this hides the number a page prints in plain text, or a dialog, a popup title
/// or the Log repeats). Not grantable: no site grant reaches it.
fn masked(result: Result<String>) -> Result<String> {
    use agentmux_common::pii_mask::mask_short_text;
    match result {
        Ok(text) => Ok(mask_short_text(&text)),
        Err(e) => Err(anyhow::anyhow!(mask_short_text(&format!("{e:#}")))),
    }
}

/// One Browser call: the action, then any follow-up it asked for.
async fn run(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let (act, args) = pick("Browser", ACTS, arguments)?;
    match act.action {
        "close" => return close(arguments, cx).await,
        "retake" => return retake(arguments, cx).await,
        "take_control" => return take_control(arguments, cx).await,
        "release" => return release(arguments, cx).await,
        "handoff_status" => return handoff_status(arguments, cx).await,
        "request_status" => return super::browser_waiting::request(arguments, cx, false).await,
        "cancel_request" => return super::browser_waiting::request(arguments, cx, true).await,
        _ => {}
    }
    // `wait`'s own conditions are the until_* parameters (pick renamed
    // them for its handler): for wait they are not a follow-up.
    // `terminal` waits for its own output (until_text, timeout_seconds), so its until_* is no follow-up either.
    let is_wait = act.action == "wait" || act.action == "terminal";
    let follow_wait =
        !is_wait && ["until_text", "until_url", "until_gone", "until_state"].iter().any(|k| arguments.get(*k).is_some());
    let follow_snapshot = arguments.get("snapshot").and_then(|v| v.as_bool()).unwrap_or(false);
    if (follow_wait || follow_snapshot) && !CHANGES_THE_PAGE.contains(&act.action) {
        anyhow::bail!(
            "until_text/until_url/until_gone/until_state and snapshot follow an action that changes the page ({}); \
             for action {:?}, call wait or snapshot on its own",
            CHANGES_THE_PAGE.join(", "),
            act.action
        );
    }
    // A page dialog the action opened is reported with it, also when the
    // action failed (a click whose confirm is waiting for the user times out).
    let mut out = match browser::call(act.tool, &args, cx).await {
        Ok(out) => out,
        Err(e) => {
            let dialogs = browser::dialogs_note(cx).await;
            return Err(if dialogs.is_empty() { e } else { anyhow::anyhow!("{e}{dialogs}") });
        }
    };
    // Waiting for the person: nothing has happened yet, so the follow-ups have nothing to wait for or read.
    if out.starts_with(super::browser_waiting::WAITING_PREFIX) {
        return Ok(out);
    }
    // The action has run: a follow-up that fails is reported after its
    // result, never instead of it, so the agent doesn't repeat a click
    // that already submitted something.
    if follow_wait {
        out = match browser::call("BrowserWaitFor", &follow_wait_args(arguments), cx).await {
            Ok(waited) => format!("{out}\n\nwait: {waited}"),
            Err(e) => format!("{out}\n\nwait failed (the {} itself was done): {e}", act.action),
        };
    }
    if follow_snapshot {
        // An action that navigates leaves the page loading: read it once it has
        // settled, not the empty page of a moment ago. A page that never settles (3 seconds at most)
        // is read as it is.
        if !follow_wait {
            let settle = json!({ "state": "idle", "timeout_seconds": 3, "pane": arguments.get("pane") });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(4), browser::call("BrowserWaitFor", &settle, cx)).await;
        }
        out = match browser::call("BrowserSnapshot", &only(arguments, &["pane", "scope", "max_text", "include_hidden"]), cx).await {
            Ok(snap) => format!("{out}\n\nsnapshot:\n{snap}"),
            Err(e) => format!("{out}\n\nsnapshot failed (the {} itself was done): {e}", act.action),
        };
    }
    out.push_str(&browser::dialogs_note(cx).await);
    Ok(out)
}

/// `close`: Stop's `pane` with the pane named. With none, ClosePane would end the caller's own session, so a
/// pane is required and there is no default.
async fn close(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let pane = arguments.get("pane").and_then(|v| v.as_str()).map(str::trim).filter(|p| !p.is_empty());
    let Some(pane) = pane.filter(|p| *p != cx.block_id) else {
        anyhow::bail!("Browser close needs pane: the id Browser open gave you. It never closes your own pane");
    };
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let req = agentmux_common::api_types::UiBrowserCloseRequest { auth: sign_ui_automation_auth()?, pane: Some(pane.to_string()) };
    let url = format!("{}/api/v1/ui/browser/close", local_url.trim_end_matches('/'));
    let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&req).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 202 {
        // The pane's user has 10 s to keep it; wait for their answer.
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        let now = await_shutdown(client, local_url, auth_key, body["request_id"].as_str().unwrap_or_default(), true).await;
        return close_pane_outcome(pane, &now);
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Browser close failed: HTTP {status} — {text}");
    }
    Ok(format!("Closed pane {pane:?}"))
}

/// `retake`: drive again a pane the user took over and gave back. The pane is required (it is not the caller's own), and
/// a snapshot comes with the answer: the user has been using the page, so what the agent last saw is out of date.
async fn retake(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let pane = arguments.get("pane").and_then(|v| v.as_str()).map(str::trim).filter(|p| !p.is_empty());
    let Some(pane) = pane else {
        anyhow::bail!("Browser retake needs pane: the id of the pane the user took over");
    };
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let req = agentmux_common::api_types::UiBrowserRetakeRequest { auth: sign_ui_automation_auth()?, pane: Some(pane.to_string()) };
    let url = format!("{}/api/v1/ui/browser/retake", local_url.trim_end_matches('/'));
    let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&req).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Browser retake failed: HTTP {status} — {text}");
    }
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let lead = if body.pointer("/data/already").and_then(|v| v.as_bool()) == Some(true) {
        format!("You already drive pane {pane:?}.")
    } else {
        format!("You drive pane {pane:?} again. The user used the page meanwhile, so read it before you act; an auto-approve you had is not back (ask again if you need it).")
    };
    match browser::call("BrowserSnapshot", &json!({ "pane": pane }), cx).await {
        Ok(snap) => Ok(format!("{lead}\n\nsnapshot:\n{snap}")),
        Err(e) => Ok(format!("{lead}\n\nsnapshot failed (the retake itself was done): {e}")),
    }
}

/// How long a take_control waits for srv: the person has 120 s to answer the pane's question, and the shared client's own
/// limit (10 s) would end the call long before.
const TAKE_CONTROL_WAIT: std::time::Duration = std::time::Duration::from_secs(150);

/// POST `body` to srv's `/api/v1/ui/browser/{route}` and read its answer, or fail with what srv said. `wait` is the time
/// limit for a call that waits for the person; none: the client's own.
async fn control_call<T: serde::Serialize>(route: &str, body: &T, wait: Option<std::time::Duration>, cx: &ToolCtx<'_>) -> Result<Value> {
    let ToolCtx { local_url, auth_key, client, .. } = *cx;
    let url = format!("{}/api/v1/ui/browser/{route}", local_url.trim_end_matches('/'));
    let mut request = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(body);
    if let Some(wait) = wait {
        request = request.timeout(wait);
    }
    let resp = request.send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Browser {route} failed: HTTP {status} — {text}");
    }
    Ok(resp.json().await.unwrap_or(Value::Null))
}

/// The pane a take_control or release names: required, since neither defaults to the caller's own.
fn named_pane(arguments: &Value, action: &str, what: &str) -> Result<String> {
    let pane = arguments.get("pane").and_then(|v| v.as_str()).map(str::trim).filter(|p| !p.is_empty());
    pane.map(str::to_string).ok_or_else(|| anyhow::anyhow!("Browser {action} needs pane: {what}"))
}

/// `take_control`: drive a browser pane the agent did not open. srv may ask the person in the pane first (their Personal
/// sign-ins, a pane another agent drives, a pane they used a moment ago) and the call waits for the answer.
async fn take_control(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let pane = named_pane(arguments, "take_control", "the id of a browser pane (Pane layout lists them)")?;
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let allowed_origins = arguments.get("allowed_origins").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|o| o.as_str().map(str::to_string)).collect());
    let req = agentmux_common::api_types::UiBrowserTakeControlRequest { auth: sign_ui_automation_auth()?, pane: Some(pane.clone()), allowed_origins };
    let body = control_call("take_control", &req, Some(TAKE_CONTROL_WAIT), cx).await?;
    let data = &body["data"];
    let lead = if data["already"].as_bool() == Some(true) {
        format!("You already drive pane {pane:?}.")
    } else {
        let limit = data["limited_to"].as_array().map(|l| l.iter().filter_map(|o| o.as_str()).collect::<Vec<_>>().join(", ")).filter(|l| !l.is_empty());
        let limit = limit.map_or_else(String::new, |l| format!(" It is limited to {l}."));
        format!(
            "You drive pane {pane:?}, which you did not open; it browses as {}.{limit} The user sees you driving it and can take it back with Take over. Read the page before you act: it was not yours until now.",
            data["browsing_as"].as_str().unwrap_or("its own profile")
        )
    };
    match browser::call("BrowserSnapshot", &json!({ "pane": pane }), cx).await {
        Ok(snap) => Ok(format!("{lead}

snapshot:
{snap}")),
        Err(e) => Ok(format!("{lead}

snapshot failed (taking control itself was done): {e}")),
    }
}

/// `release`: stop driving a pane, so that it has no driver and another agent can take it without asking the person.
async fn release(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let pane = named_pane(arguments, "release", "the id of a pane you drive")?;
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let req = agentmux_common::api_types::UiBrowserReleaseRequest { auth: sign_ui_automation_auth()?, pane: Some(pane.clone()) };
    control_call("release", &req, None, cx).await?;
    Ok(format!("You no longer drive pane {pane:?}; it stays open for whoever uses it next."))
}

/// `handoff_status`: what became of a hand-off, by its id or the newest on a pane.
async fn handoff_status(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let text_arg = |k: &str| arguments.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (handoff_id, pane) = (text_arg("handoff_id"), text_arg("pane"));
    if handoff_id.is_none() && pane.is_none() {
        anyhow::bail!("Browser handoff_status needs handoff_id (the hand-off's answer gave it) or pane");
    }
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let req = agentmux_common::api_types::UiBrowserHandoffStatusRequest { auth: sign_ui_automation_auth()?, pane, handoff_id };
    let url = format!("{}/api/v1/ui/browser/handoff_status", local_url.trim_end_matches('/'));
    let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&req).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Browser handoff_status failed: HTTP {status} — {text}");
    }
    let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!("Browser handoff_status: bad response: {e}"))?;
    Ok(handoff_status_text(body.get("data").unwrap_or(&Value::Null)))
}

/// What the agent reads for a hand-off's status.
fn handoff_status_text(d: &Value) -> String {
    let id = d["handoff_id"].as_str().unwrap_or("?");
    let pane = d["pane"].as_str().unwrap_or("?");
    let ago = |k: &str| d[k].as_u64().map(|s| if s < 90 { format!("{s} s ago") } else { format!("{} min ago", s / 60) });
    match d["status"].as_str().unwrap_or("") {
        "pending" => format!("Hand-off {id} on pane {pane:?} is still waiting for the user (asked {}).", ago("started_secs_ago").unwrap_or_default()),
        "done" => format!("Hand-off {id}: the user clicked Done ({}). Take a new snapshot: don't assume what they changed.", ago("ended_secs_ago").unwrap_or_default()),
        "cancelled" => format!("Hand-off {id}: the user clicked Cancel ({}). Ask them in chat how to proceed.", ago("ended_secs_ago").unwrap_or_default()),
        "timed_out" => format!("Hand-off {id}: the user didn't answer in time ({}). Ask them in chat.", ago("ended_secs_ago").unwrap_or_default()),
        _ => format!(
            "Hand-off {id}: abandoned ({}): your call ended before the user answered, so its request went down with it and nobody can answer it. Make a new hand-off if you still need the user.",
            ago("ended_secs_ago").unwrap_or_default()
        ),
    }
}

/// The wait that follows an action, from its `until_*` parameters only: the
/// action's own `text` (a fill's value) is not a wait condition.
fn follow_wait_args(arguments: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (ours, theirs) in [
        ("pane", "pane"),
        ("until_text", "text"),
        ("until_url", "url_contains"),
        ("until_gone", "gone"),
        ("until_state", "state"),
        ("timeout_seconds", "timeout_seconds"),
    ] {
        if let Some(v) = arguments.get(ours) {
            out.insert(theirs.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

/// `arguments` with only `keys`, for a follow-up handler that checks what it's given.
fn only(arguments: &Value, keys: &[&str]) -> Value {
    let mut out = serde_json::Map::new();
    for k in keys {
        if let Some(v) = arguments.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx_parts() -> (reqwest::Client, LoopRegistry, std::sync::atomic::AtomicU64) {
        (reqwest::Client::new(), LoopRegistry::default(), std::sync::atomic::AtomicU64::new(0))
    }

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::BROWSER_TOOL).unwrap();
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
    fn wait_takes_the_until_parameters() {
        let (act, args) = pick("Browser", ACTS, &json!({ "action": "wait", "until_url": "/done", "pane": "p" })).unwrap();
        assert_eq!(act.tool, "BrowserWaitFor");
        assert_eq!(args, json!({ "url_contains": "/done", "pane": "p" }));
        assert_eq!(only(&json!({ "a": 1, "b": 2 }), &["b", "c"]), json!({ "b": 2 }));
    }

    #[test]
    fn wait_takes_until_state_and_a_follow_up_wait_passes_it_on() {
        let (_, args) = pick("Browser", ACTS, &json!({ "action": "wait", "until_state": "idle", "pane": "p" })).unwrap();
        assert_eq!(args, json!({ "state": "idle", "pane": "p" }));
        let args = json!({ "action": "click", "pane": "p", "ref": "e2", "until_state": "load" });
        assert_eq!(follow_wait_args(&args), json!({ "pane": "p", "state": "load" }));
    }

    #[test]
    fn a_follow_up_wait_uses_the_until_parameters_not_the_actions_text() {
        let args = json!({ "action": "fill", "pane": "p", "ref": "e2", "text": "hello", "until_url": "/next", "timeout_seconds": 5 });
        assert_eq!(follow_wait_args(&args), json!({ "pane": "p", "url_contains": "/next", "timeout_seconds": 5 }));
    }

    /// wait with its conditions reaches its handler (which, with no srv here,
    /// fails on the missing environment) instead of the follow-up guard.
    #[tokio::test]
    async fn wait_with_its_conditions_is_not_taken_for_a_follow_up() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        for args in [
            json!({ "action": "wait", "until_text": "Done" }),
            json!({ "action": "wait", "until_url": "/next", "pane": "p" }),
            json!({ "action": "wait", "until_gone": "e4" }),
        ] {
            let e = call("Browser", &args, &cx).await.unwrap_err().to_string();
            assert!(!e.contains("follow an action that changes the page"), "{args}: {e}");
        }
    }

    /// ClosePane with no pane ends the caller's own session, so `close` must name one: no default, even the
    /// caller's own pane, and a blank id is none.
    #[tokio::test]
    async fn close_needs_a_pane_and_never_defaults_to_the_callers_own() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "my-block", client: &client, loops: &loops, loop_counter: &counter };
        for args in [json!({ "action": "close" }), json!({ "action": "close", "pane": "  " })] {
            let e = call("Browser", &args, &cx).await.unwrap_err().to_string();
            assert!(e.contains("needs pane") && e.contains("never closes your own pane"), "{args}: {e}");
        }
        let own = call("Browser", &json!({ "action": "close", "pane": "my-block" }), &cx).await.unwrap_err().to_string();
        assert!(own.contains("never closes your own pane"), "{own}");
        // With a pane it reaches ClosePane (which, with no srv here, fails on the missing environment).
        let e = call("Browser", &json!({ "action": "close", "pane": "p1" }), &cx).await.unwrap_err().to_string();
        assert!(!e.contains("needs pane"), "{e}");
    }

    /// The person has 120 s to answer a take_control question (srv's `ASK_FOR`): the call must outlast it, and the shared
    /// client's 10 s limit must not be the one that applies.
    #[test]
    fn a_take_control_waits_longer_than_the_person_has_to_answer() {
        assert!(TAKE_CONTROL_WAIT > std::time::Duration::from_secs(120));
        assert!(TAKE_CONTROL_WAIT > std::time::Duration::from_secs(10));
    }

    /// Taking control and releasing name the pane they act on: neither defaults to the caller's own.
    #[tokio::test]
    async fn take_control_and_release_need_a_pane() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "my-block", client: &client, loops: &loops, loop_counter: &counter };
        for action in ["take_control", "release"] {
            let e = call("Browser", &json!({ "action": action }), &cx).await.unwrap_err().to_string();
            assert!(e.contains(&format!("{action} needs pane")), "{e}");
            let e = call("Browser", &json!({ "action": action, "pane": "p1" }), &cx).await.unwrap_err().to_string();
            assert!(!e.contains("needs pane"), "{e}");
        }
    }

    /// A retake names the pane it takes back and a status names the hand-off or the pane: neither defaults to the caller's own.
    #[tokio::test]
    async fn retake_and_handoff_status_need_something_to_name() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "my-block", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Browser", &json!({ "action": "retake", "pane": "  " }), &cx).await.unwrap_err().to_string();
        assert!(e.contains("retake needs pane"), "{e}");
        let e = call("Browser", &json!({ "action": "handoff_status" }), &cx).await.unwrap_err().to_string();
        assert!(e.contains("needs handoff_id"), "{e}");
        // With one they reach srv (which, with none here, fails on the missing environment).
        let e = call("Browser", &json!({ "action": "retake", "pane": "p1" }), &cx).await.unwrap_err().to_string();
        assert!(!e.contains("needs pane"), "{e}");
        let e = call("Browser", &json!({ "action": "handoff_status", "pane": "p1" }), &cx).await.unwrap_err().to_string();
        assert!(!e.contains("needs handoff_id"), "{e}");
    }

    #[test]
    fn a_hand_off_status_says_what_to_do_next() {
        let d = |status: &str| json!({ "handoff_id": "h1", "pane": "p1", "status": status, "started_secs_ago": 30, "ended_secs_ago": 200 });
        assert!(handoff_status_text(&d("pending")).contains("still waiting for the user (asked 30 s ago)"));
        assert!(handoff_status_text(&d("done")).contains("clicked Done (3 min ago)") && handoff_status_text(&d("done")).contains("snapshot"));
        assert!(handoff_status_text(&d("cancelled")).contains("clicked Cancel"));
        assert!(handoff_status_text(&d("timed_out")).contains("didn't answer in time"));
        let abandoned = handoff_status_text(&d("abandoned"));
        assert!(abandoned.contains("abandoned") && abandoned.contains("nobody can answer it") && abandoned.contains("new hand-off"), "{abandoned}");
    }

    #[tokio::test]
    async fn a_follow_up_is_refused_after_an_action_that_reads() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Browser", &json!({ "action": "eval", "script": "1", "snapshot": true }), &cx).await.unwrap_err().to_string();
        assert!(e.contains("follow an action that changes the page"), "{e}");
    }
}

#[cfg(test)]
mod mask_tests {
    use super::*;

    #[test]
    fn every_answer_and_error_has_card_numbers_and_codes_hidden() {
        let ok = masked(Ok("- text \"Paid with 4111 1111 1111 1111, CVV 123\"
".into())).unwrap();
        assert!(!ok.contains("4111") && ok.contains("[card number hidden]") && ok.contains("[code hidden]"), "{ok}");
        let err = masked(Err(anyhow::anyhow!("the page said: your verification code is 482913"))).unwrap_err().to_string();
        assert!(!err.contains("482913") && err.contains("[code hidden]"), "{err}");
        assert_eq!(masked(Ok("pane 7f3a2c: done".into())).unwrap(), "pane 7f3a2c: done");
    }
}
