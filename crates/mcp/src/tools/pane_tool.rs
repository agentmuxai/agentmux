// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane: one tool for the agent's place in the UI and the panes it shows the
//! user, with an `action` (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md
//! §4.2, slice S4). It replaces WhoAmI, Layout, OpenEditor, OpenMedia,
//! OpenFiles, NewTab, SetActiveTab, FocusWindow and SetName; their handlers
//! (panes.rs) run each action unchanged. Closing a pane is in Stop.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTIONS: &[&str] = &["whoami", "layout", "open", "new_tab", "activate_tab", "focus_window", "rename", "resize", "move", "place"];

const ACTS: &[Act] = &[
    Act { action: "whoami", tool: "WhoAmI", rename: &[] },
    Act { action: "layout", tool: "Layout", rename: &[] },
    Act { action: "new_tab", tool: "NewTab", rename: &[] },
    Act { action: "activate_tab", tool: "SetActiveTab", rename: &[] },
    Act { action: "focus_window", tool: "FocusWindow", rename: &[] },
    Act { action: "rename", tool: "SetName", rename: &[] },
    Act { action: "resize", tool: "ResizePane", rename: &[] },
    Act { action: "move", tool: "MovePane", rename: &[] },
    Act { action: "place", tool: "PlacePane", rename: &[] },
];

/// `open`'s handler for each view; the editor and media handlers take the
/// path as `file`.
const OPEN: &[(&str, Act)] = &[
    ("editor", Act { action: "open", tool: "OpenEditor", rename: &[("path", "file")] }),
    ("media", Act { action: "open", tool: "OpenMedia", rename: &[("path", "file")] }),
    ("files", Act { action: "open", tool: "OpenFiles", rename: &[] }),
];

/// `open` with a deep link, or a view at a place (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R16):
/// srv's guard decides it.
const OPEN_LINK: Act = Act { action: "open", tool: "OpenLink", rename: &[] };

/// Whether a Pane open is a deep link: it names `link`, or a view that is not a file view given a path.
fn is_link_open(arguments: &Value) -> bool {
    let has = |k: &str| arguments.get(k).is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    let view = arguments.get("view").and_then(|v| v.as_str()).unwrap_or("");
    has("link") || (!view.is_empty() && !has("path") && (has("place") || !OPEN.iter().any(|(v, _)| *v == view)))
}

/// The handler for a Pane call (as the one-action list it was picked from,
/// for [`relabeled`]) and its arguments as that handler takes them.
fn route(arguments: &Value) -> Result<(&'static [Act], Value)> {
    let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "open" if is_link_open(arguments) => {
            let acts = std::slice::from_ref(&OPEN_LINK);
            let (_, args) = pick("Pane", acts, arguments)?;
            Ok((acts, args))
        }
        "open" => {
            let view = arguments.get("view").and_then(|v| v.as_str()).unwrap_or("");
            let Some((_, act)) = OPEN.iter().find(|(v, _)| *v == view) else {
                anyhow::bail!("Pane open needs link (agentmux://view/place, every view and place AgentMux has), or view: \"editor\" (a file), \"media\" (an image, video or audio file) or \"files\" (a folder) with a path");
            };
            // Checked here: the editor and media handlers call it `file`,
            // which Pane doesn't take.
            if arguments.get("path").and_then(|v| v.as_str()).map_or(true, |p| p.trim().is_empty()) {
                anyhow::bail!("Pane open: missing required parameter: path (absolute)");
            }
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
        assert!(e.contains("needs link"), "{e}");
        for view in ["editor", "media", "files"] {
            let e = route(&json!({ "action": "open", "view": view })).unwrap_err().to_string();
            assert_eq!(e, "Pane open: missing required parameter: path (absolute)");
        }
    }

    #[test]
    fn a_link_or_a_view_at_a_place_is_a_deep_link_open() {
        for args in [
            json!({ "action": "open", "link": "agentmux://settings/recording" }),
            json!({ "action": "open", "view": "settings", "place": "recording" }),
            json!({ "action": "open", "view": "vault" }),
            json!({ "action": "open", "view": "editor", "place": "C:/w/a.md" }),
        ] {
            assert_eq!(route(&args).unwrap().0[0].tool, "OpenLink", "{args}");
        }
        // A file view with a path keeps its own handler (a container agent's snapshot, the split).
        assert_eq!(route(&json!({ "action": "open", "view": "editor", "path": "/w/a.rs" })).unwrap().0[0].tool, "OpenEditor");
    }

    /// The schema's views are the deep-link registry's views of this build, so a model is offered every place
    /// srv can open and none it can't.
    #[test]
    fn the_views_offered_are_the_registrys_of_this_build() {
        use agentmux_common::deeplinks::REGISTRY;
        let schema: Value = serde_json::from_str(crate::tool_schemas::PANE_TOOL).unwrap();
        let mut offered: Vec<&str> = schema["inputSchema"]["properties"]["view"]["enum"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        let mut openable: Vec<&str> = REGISTRY.views.iter().filter(|v| v.build <= REGISTRY.build && !v.prefix).map(|v| v.id).collect();
        offered.sort();
        openable.sort();
        assert_eq!(offered, openable);
    }

    #[test]
    fn a_link_is_written_in_its_one_form_from_a_view_a_place_and_parameters() {
        use super::super::panes::link_of;
        assert_eq!(link_of(&json!({ "link": " agentmux://help " })).unwrap(), "agentmux://help");
        assert_eq!(link_of(&json!({ "view": "settings", "place": "recording", "anchor": "recording.engine" })).unwrap(), "agentmux://settings/recording#recording.engine");
        assert_eq!(link_of(&json!({ "view": "kanban", "place": "task/48139" })).unwrap(), "agentmux://kanban/task/48139");
        // Parameters in the registry's order whatever order they came in; a number is written as its digits.
        assert_eq!(link_of(&json!({ "view": "editor", "place": "/w/a.md", "params": { "offset": 4, "line": 2 } })).unwrap(), "agentmux://editor/%2Fw%2Fa.md?line=2&offset=4");
        assert!(link_of(&json!({})).is_err());
    }

    #[test]
    fn the_answer_says_what_happened_and_for_a_refusal_what_would_work() {
        use super::super::panes::describe_open_outcome;
        let opened = describe_open_outcome(&json!({ "outcome": "opened", "label": "settings › recording", "block_id": "b1", "link": "agentmux://settings/recording" }));
        assert_eq!(opened, "Opened settings › recording beside this conversation (block b1). Link: agentmux://settings/recording.");
        let refused = describe_open_outcome(&json!({ "outcome": "refused", "reason": "an agent may open only its own place of audit", "remedy": "agentmux://audit/agent/a1" }));
        assert_eq!(refused, "Not opened: an agent may open only its own place of audit. agentmux://audit/agent/a1.");
    }

    #[test]
    fn the_other_actions_run_their_handlers() {
        let (acts, args) = route(&json!({ "action": "rename", "target": "tab", "name": "x" })).unwrap();
        assert_eq!(acts[0].tool, "SetName");
        assert_eq!(args, json!({ "target": "tab", "name": "x" }));
        // A file's rename carries its path to the same handler.
        let (acts, args) = route(&json!({ "action": "rename", "target": "file", "path": "/w/a.rs", "name": "b.rs" })).unwrap();
        assert_eq!(acts[0].tool, "SetName");
        assert_eq!(args, json!({ "target": "file", "path": "/w/a.rs", "name": "b.rs" }));
        assert_eq!(route(&json!({ "action": "whoami" })).unwrap().0[0].tool, "WhoAmI");
        // A floating pane's size and place: each action has its own handler, since the handler is told which by its name.
        for (action, tool) in [("resize", "ResizePane"), ("move", "MovePane"), ("place", "PlacePane")] {
            let (acts, args) = route(&json!({ "action": action, "target_id": "b1", "width": 700 })).unwrap();
            assert_eq!(acts[0].tool, tool);
            assert_eq!(args, json!({ "target_id": "b1", "width": 700 }));
        }
        let e = route(&json!({ "action": "close" })).unwrap_err().to_string();
        assert_eq!(e, format!("Pane: unknown action \"close\": use one of {}", ACTIONS.join(", ")));
    }
}
