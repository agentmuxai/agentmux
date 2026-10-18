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
    Act { action: "click", tool: "BrowserClick", rename: &[] },
    Act { action: "fill", tool: "BrowserFill", rename: &[] },
    Act { action: "select", tool: "BrowserSelect", rename: &[] },
    Act { action: "check", tool: "BrowserCheck", rename: &[] },
    Act { action: "set_files", tool: "BrowserSetFiles", rename: &[] },
    Act { action: "navigate", tool: "BrowserNavigate", rename: &[] },
    Act { action: "back", tool: "BrowserBack", rename: &[] },
    Act { action: "forward", tool: "BrowserForward", rename: &[] },
    Act { action: "reload", tool: "BrowserReload", rename: &[] },
    Act { action: "key", tool: "BrowserDispatchKey", rename: &[] },
    Act { action: "focus", tool: "BrowserFocusElement", rename: &[] },
    Act { action: "focus_info", tool: "BrowserFocusInfo", rename: &[] },
    Act {
        action: "wait",
        tool: "BrowserWaitFor",
        rename: &[("until_text", "text"), ("until_url", "url_contains"), ("until_gone", "gone")],
    },
    Act { action: "eval", tool: "BrowserEval", rename: &[] },
    Act { action: "handoff", tool: "BrowserHandoff", rename: &[] },
    Act { action: "request_auto_approve", tool: "BrowserRequestBypass", rename: &[] },
    Act { action: "stop_auto_approve", tool: "BrowserEndBypass", rename: &[] },
    Act { action: "log", tool: "BrowserLog", rename: &[] },
];

/// Actions after which `until_*` and `snapshot` may follow in the same call.
const CHANGES_THE_PAGE: &[&str] = &["click", "fill", "select", "check", "key", "navigate", "back", "forward", "reload"];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Browser" => relabeled("Browser", ACTS, run(arguments, cx).await),
        _ => Err(not_in_family()),
    }
}

/// One Browser call: the action, then any follow-up it asked for.
async fn run(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let (act, args) = pick("Browser", ACTS, arguments)?;
    // `wait`'s own conditions are the until_* parameters (pick renamed
    // them for its handler): for wait they are not a follow-up.
    let is_wait = act.action == "wait";
    let follow_wait =
        !is_wait && ["until_text", "until_url", "until_gone"].iter().any(|k| arguments.get(*k).is_some());
    let follow_snapshot = arguments.get("snapshot").and_then(|v| v.as_bool()).unwrap_or(false);
    if (follow_wait || follow_snapshot) && !CHANGES_THE_PAGE.contains(&act.action) {
        anyhow::bail!(
            "until_text/until_url/until_gone and snapshot follow an action that changes the page ({}); \
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
        out = match browser::call("BrowserSnapshot", &only(arguments, &["pane", "scope"]), cx).await {
            Ok(snap) => format!("{out}\n\nsnapshot:\n{snap}"),
            Err(e) => format!("{out}\n\nsnapshot failed (the {} itself was done): {e}", act.action),
        };
    }
    out.push_str(&browser::dialogs_note(cx).await);
    Ok(out)
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

    #[tokio::test]
    async fn a_follow_up_is_refused_after_an_action_that_reads() {
        let (client, loops, counter) = cx_parts();
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Browser", &json!({ "action": "eval", "script": "1", "snapshot": true }), &cx).await.unwrap_err().to_string();
        assert!(e.contains("follow an action that changes the page"), "{e}");
    }
}
