// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane: one tool for the agent's place in the UI and the panes it shows the
//! user, with an `action` (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md
//! §4.2, slice S4). It replaces WhoAmI, Layout, OpenEditor, OpenMedia,
//! OpenFiles, NewTab, SetActiveTab, FocusWindow and SetName; their handlers
//! (panes.rs) run each action unchanged. Closing a pane is in Stop.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTIONS: &[&str] = &["whoami", "layout", "open", "new_tab", "activate_tab", "focus_window", "rename"];

const ACTS: &[Act] = &[
    Act { action: "whoami", tool: "WhoAmI", rename: &[] },
    Act { action: "layout", tool: "Layout", rename: &[] },
    Act { action: "new_tab", tool: "NewTab", rename: &[] },
    Act { action: "activate_tab", tool: "SetActiveTab", rename: &[] },
    Act { action: "focus_window", tool: "FocusWindow", rename: &[] },
    Act { action: "rename", tool: "SetName", rename: &[] },
];

/// `open`'s handler for each view; the editor and media handlers take the
/// path as `file`.
const OPEN: &[(&str, Act)] = &[
    ("editor", Act { action: "open", tool: "OpenEditor", rename: &[("path", "file")] }),
    ("media", Act { action: "open", tool: "OpenMedia", rename: &[("path", "file")] }),
    ("files", Act { action: "open", tool: "OpenFiles", rename: &[] }),
];

/// The handler for a Pane call (as the one-action list it was picked from,
/// for [`relabeled`]) and its arguments as that handler takes them.
fn route(arguments: &Value) -> Result<(&'static [Act], Value)> {
    let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "open" => {
            let view = arguments.get("view").and_then(|v| v.as_str()).unwrap_or("");
            let Some((_, act)) = OPEN.iter().find(|(v, _)| *v == view) else {
                anyhow::bail!("Pane open needs view: \"editor\" (a file), \"media\" (an image, video or audio file) or \"files\" (a folder)");
            };
            let acts = std::slice::from_ref(act);
            let (_, mut args) = pick("Pane", acts, arguments)?;
            if let Value::Object(m) = &mut args {
                m.remove("view");
            }
            Ok((acts, args))
        }
        "" => anyhow::bail!("Pane: missing required parameter: action (one of {})", ACTIONS.join(", ")),
        a if ACTIONS.contains(&a) => {
            let (act, args) = pick("Pane", ACTS, arguments)?;
            Ok((std::slice::from_ref(act), args))
        }
        other => anyhow::bail!("Pane: unknown action {other:?}: use one of {}", ACTIONS.join(", ")),
    }
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Pane" => {
            let (acts, args) = route(arguments)?;
            relabeled("Pane", acts, panes::call(acts[0].tool, &args, cx).await)
        }
        // Still its own tool (a container agent's only one of its kind); its
        // handler sits with the pane handlers.
        "RegisterDevServer" => panes::call(name, arguments, cx).await,
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::PANE_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let mut want = ACTIONS.to_vec();
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
        for act in ACTS {
            assert!(ACTIONS.contains(&act.action), "{}", act.action);
        }
    }

    #[test]
    fn open_runs_the_handler_for_its_view() {
        let (acts, args) = route(&json!({ "action": "open", "view": "editor", "path": "/w/a.rs", "split": "down" })).unwrap();
        assert_eq!(acts[0].tool, "OpenEditor");
        assert_eq!(args, json!({ "file": "/w/a.rs", "split": "down" }));
        let (acts, args) = route(&json!({ "action": "open", "view": "files", "path": "/w", "select": ["a"] })).unwrap();
        assert_eq!(acts[0].tool, "OpenFiles");
        assert_eq!(args, json!({ "path": "/w", "select": ["a"] }));
        let e = route(&json!({ "action": "open", "path": "/w" })).unwrap_err().to_string();
        assert!(e.contains("needs view"), "{e}");
    }

    #[test]
    fn the_other_actions_run_their_handlers() {
        let (acts, args) = route(&json!({ "action": "rename", "target": "tab", "name": "x" })).unwrap();
        assert_eq!(acts[0].tool, "SetName");
        assert_eq!(args, json!({ "target": "tab", "name": "x" }));
        assert_eq!(route(&json!({ "action": "whoami" })).unwrap().0[0].tool, "WhoAmI");
        let e = route(&json!({ "action": "close" })).unwrap_err().to_string();
        assert_eq!(e, format!("Pane: unknown action \"close\": use one of {}", ACTIONS.join(", ")));
    }
}
