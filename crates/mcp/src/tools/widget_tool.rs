// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Widget: one tool for the custom panes an agent builds for the user, with an
//! `action` (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice
//! S4). It replaces WidgetList, WidgetInstall and OpenWidget; their handlers
//! (widgets.rs) run each action unchanged.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "list", tool: "WidgetList", rename: &[] },
    Act { action: "install", tool: "WidgetInstall", rename: &[] },
    Act { action: "open", tool: "OpenWidget", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Widget" => {
            let (act, args) = pick("Widget", ACTS, arguments)?;
            relabeled("Widget", ACTS, widgets::call(act.tool, &args, cx).await)
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::WIDGET_TOOL).unwrap();
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
}
