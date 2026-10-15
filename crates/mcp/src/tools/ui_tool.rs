// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! UI: one tool for inspecting and driving AgentMux's own UI, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S2). It
//! replaces UIScreenshot, UIQuery, UIClick, ListShortcuts, RunCommand,
//! PressKeys, CaptureWindow and DiscoverWindows; their handlers (ui.rs) run
//! each action unchanged.

use super::family::{pick, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "screenshot", tool: "UIScreenshot", rename: &[] },
    Act { action: "query", tool: "UIQuery", rename: &[] },
    Act { action: "click", tool: "UIClick", rename: &[] },
    Act { action: "shortcuts", tool: "ListShortcuts", rename: &[] },
    Act { action: "command", tool: "RunCommand", rename: &[] },
    Act { action: "keys", tool: "PressKeys", rename: &[] },
    Act { action: "windows", tool: "DiscoverWindows", rename: &[] },
    Act { action: "window", tool: "CaptureWindow", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "UI" => {
            let (act, args) = pick("UI", ACTS, arguments)?;
            ui::call(act.tool, &args, cx).await
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::UI_TOOL).unwrap();
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
