// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `call_tool`'s handlers, one file per tool family
//! (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2 (a)).
//! Each family's `call` handles the tools it owns and returns
//! `not_in_family()` for any other name, so no tool list is written twice.

use super::*;

mod accounts;
mod browser;
mod cron;
mod fleet;
mod global_memory;
mod history;
mod loops;
mod memory;
mod panes;
mod presets;
mod pty_shell;
mod shell;
mod ui;
mod widgets;
mod work;

/// What every handler may need from the MCP process.
#[derive(Clone, Copy)]
pub(crate) struct ToolCtx<'a> {
    pub(crate) local_url: &'a str,
    pub(crate) auth_key: &'a str,
    pub(crate) block_id: &'a str,
    pub(crate) client: &'a reqwest::Client,
    pub(crate) loops: &'a LoopRegistry,
    pub(crate) loop_counter: &'a AtomicU64,
}

/// The optional `pane` argument: a browser pane this agent opened with
/// `OpenBrowser`. srv checks ownership; absent = the agent's own pane.
fn pane_arg(arguments: &Value) -> Option<String> {
    arguments
        .get("pane")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A family's answer for a tool it doesn't own.
#[derive(Debug)]
struct NotInFamily;

impl std::fmt::Display for NotInFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not in this tool family")
    }
}

impl std::error::Error for NotInFamily {}

fn not_in_family() -> anyhow::Error {
    anyhow::Error::new(NotInFamily)
}

/// Try each family in turn; the first that owns `name` answers.
pub(crate) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    macro_rules! route {
        ($($family:ident),*) => {$(
            match $family::call(name, arguments, cx).await {
                Err(e) if e.is::<NotInFamily>() => {}
                answered => return answered,
            }
        )*};
    }
    route!(
        shell,
        pty_shell,
        panes,
        fleet,
        history,
        ui,
        browser,
        loops,
        work,
        cron,
        memory,
        global_memory,
        presets,
        accounts,
        widgets
    );
    anyhow::bail!("unknown tool: {name}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    const FAMILIES: &[(&str, &str)] = &[
        ("accounts", include_str!("accounts.rs")),
        ("browser", include_str!("browser.rs")),
        ("cron", include_str!("cron.rs")),
        ("fleet", include_str!("fleet.rs")),
        ("global_memory", include_str!("global_memory.rs")),
        ("history", include_str!("history.rs")),
        ("loops", include_str!("loops.rs")),
        ("memory", include_str!("memory.rs")),
        ("panes", include_str!("panes.rs")),
        ("presets", include_str!("presets.rs")),
        ("pty_shell", include_str!("pty_shell.rs")),
        ("shell", include_str!("shell.rs")),
        ("ui", include_str!("ui.rs")),
        ("widgets", include_str!("widgets.rs")),
        ("work", include_str!("work.rs")),
    ];

    /// The tool names in a family's `match name` arms (`"A" | "B" => {`).
    fn arm_names(src: &str) -> Vec<String> {
        src.lines()
            .filter_map(|l| l.strip_prefix("        \""))
            .filter_map(|l| l.strip_suffix(" => {"))
            .flat_map(|pat| pat.split(" | ").map(|n| n.trim_matches('"').to_string()))
            .collect()
    }

    /// Every tool `tools/list` can advertise has exactly one handler, and
    /// every handler belongs to an advertised tool. A name in two families
    /// would make the later one unreachable.
    #[test]
    fn every_advertised_tool_has_exactly_one_handler() {
        let mut owner: BTreeMap<String, &str> = BTreeMap::new();
        for (family, src) in FAMILIES {
            let names = arm_names(src);
            assert!(!names.is_empty(), "{family} has no arms");
            for n in names {
                if let Some(prev) = owner.insert(n.clone(), family) {
                    panic!("{n} is handled by both {prev} and {family}");
                }
            }
        }
        let schemas = include_str!("../tool_schemas.rs");
        let advertised: Vec<String> = schemas
            .lines()
            .filter_map(|l| l.trim().strip_prefix("\"name\": \""))
            .filter_map(|l| l.split('"').next())
            .map(str::to_string)
            .collect();
        assert!(!advertised.is_empty());
        for n in &advertised {
            assert!(owner.contains_key(n), "{n} is advertised but has no handler");
        }
        for n in owner.keys() {
            assert!(advertised.contains(n), "{n} has a handler but no schema");
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_is_reported_by_name() {
        let client = reqwest::Client::new();
        let loops = super::LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = super::ToolCtx {
            local_url: "",
            auth_key: "",
            block_id: "",
            client: &client,
            loops: &loops,
            loop_counter: &counter,
        };
        let err = super::call("NoSuchTool", &serde_json::json!({}), &cx)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown tool: NoSuchTool");
    }
}
