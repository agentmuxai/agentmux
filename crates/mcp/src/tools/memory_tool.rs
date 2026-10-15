// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Memory: one tool for an agent's Personal Memory and the workspace's Global
//! Memory, with an `action` and a `scope`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S3). It
//! replaces the six Memory* and seven GlobalMemory* tools; their handlers
//! (memory.rs, global_memory.rs) run each action unchanged.

use super::*;

pub(super) const ACTIONS: &[&str] = &["list", "read", "write", "remove", "history", "diff", "revert"];

/// The handler for `action` at `scope`, and the parameters it calls
/// differently. Personal Memory has no `remove`; its revert names the version
/// `target_version_id`, Global's `version_id`.
pub(super) fn handler(action: &str, scope: &str) -> Result<(&'static str, &'static [(&'static str, &'static str)])> {
    Ok(match (scope, action) {
        ("personal", "list") => ("MemoryList", &[]),
        ("personal", "read") => ("MemoryRead", &[]),
        ("personal", "write") => ("MemoryWrite", &[]),
        ("personal", "history") => ("MemoryHistory", &[]),
        ("personal", "diff") => ("MemoryDiff", &[]),
        ("personal", "revert") => ("MemoryRevert", &[("version_id", "target_version_id")]),
        ("personal", "remove") => anyhow::bail!(
            "Memory: remove is for scope \"global\" (taking an entry out of Global Memory); a Personal Memory file is \
             rewritten with write"
        ),
        ("global", "list") => ("GlobalMemoryList", &[]),
        ("global", "read") => ("GlobalMemoryRead", &[]),
        ("global", "write") => ("GlobalMemoryWrite", &[]),
        ("global", "remove") => ("GlobalMemoryRemove", &[]),
        ("global", "history") => ("GlobalMemoryHistory", &[]),
        ("global", "diff") => ("GlobalMemoryDiff", &[]),
        ("global", "revert") => ("GlobalMemoryRevert", &[]),
        ("personal" | "global", "") => {
            anyhow::bail!("Memory: missing required parameter: action (one of {})", ACTIONS.join(", "))
        }
        ("personal" | "global", other) => {
            anyhow::bail!("Memory: unknown action {other:?}: use one of {}", ACTIONS.join(", "))
        }
        (other, _) => anyhow::bail!("Memory: unknown scope {other:?}: use \"personal\" or \"global\""),
    })
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Memory" => {
            let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let scope = arguments.get("scope").and_then(|v| v.as_str()).unwrap_or("personal");
            let (tool, rename) = handler(action, scope)?;
            let mut args = match arguments {
                Value::Object(m) => m.clone(),
                _ => serde_json::Map::new(),
            };
            args.remove("action");
            args.remove("scope");
            for (ours, theirs) in rename {
                if let Some(v) = args.remove(*ours) {
                    args.insert((*theirs).to_string(), v);
                }
            }
            let args = Value::Object(args);
            match scope {
                "global" => global_memory::call(tool, &args, cx).await,
                _ => memory::call(tool, &args, cx).await,
            }
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_action_and_scope_has_its_handler() {
        assert_eq!(handler("list", "personal").unwrap().0, "MemoryList");
        assert_eq!(handler("write", "global").unwrap().0, "GlobalMemoryWrite");
        assert_eq!(handler("revert", "personal").unwrap(), ("MemoryRevert", &[("version_id", "target_version_id")][..]));
        assert_eq!(handler("revert", "global").unwrap().0, "GlobalMemoryRevert");
        for a in ACTIONS {
            assert!(handler(a, "global").is_ok(), "{a}");
        }
        assert!(handler("remove", "personal").unwrap_err().to_string().contains("scope \"global\""));
        assert!(handler("drop", "personal").unwrap_err().to_string().contains("unknown action"));
        assert!(handler("list", "team").unwrap_err().to_string().contains("unknown scope"));
    }

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::MEMORY_TOOL).unwrap();
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
    }
}
