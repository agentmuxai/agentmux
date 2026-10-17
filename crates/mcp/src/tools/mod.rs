// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `call_tool`'s handlers, one file per tool family
//! (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2 (a)).
//! Each family's `call` handles the tools it owns and returns
//! `not_in_family()` for any other name, so no tool list is written twice.

use super::*;

mod accounts;
mod agents_tool;
mod app_tool;
mod browser;
mod browser_tool;
mod cron;
mod family;
mod fleet;
mod global_memory;
mod history;
mod identity_tool;
mod loops;
mod memory;
mod memory_tool;
mod message_tool;
mod open_prs;
mod overlap;
mod pane_tool;
mod panes;
mod presence_tool;
mod presets;
mod pty_shell;
mod schedule_tool;
mod send_files;
mod shell;
mod stop_tool;
mod tasks_tool;
mod terminal;
mod ui;
mod ui_tool;
mod widget_tool;
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

/// A path the agent gave, made absolute against its own workspace
/// (`AGENTMUX_AGENT_WORKDIR`, else this process's directory). It doesn't
/// follow a `cd` in the agent's shell, so tools ask for absolute paths.
fn resolve_workdir_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    let base = std::env::var("AGENTMUX_AGENT_WORKDIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    base.join(p).to_string_lossy().into_owned()
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

/// Tools a family tool replaced (SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md
/// §6): the model only sees listed tools, but a memory, a skill or a script
/// may still name an old one, so the error says what to use instead.
const REPLACED: &[(&str, &str)] = &[
    ("Shell", "Terminal with action \"run\" and background: true"),
    ("ShellInput", "Terminal with action \"type\""),
    ("ShellStatus", "Terminal with action \"wait\" and until \"exit\", or \"read\""),
    ("ShellStop", "Terminal with action \"close\""),
    ("PtyShell", "Terminal with action \"open\""),
    ("PtyShellInput", "Terminal with action \"type\" (keys by name; submit presses Enter)"),
    ("PtyShellResize", "Terminal with action \"type\" and rows/cols"),
    ("PtyShellRead", "Terminal with action \"read\""),
    ("PtyShellStatus", "Terminal with action \"read\" (running, exit_code)"),
    ("PtyShellStop", "Terminal with action \"close\""),
    ("ConnList", "Terminal with action \"connections\""),
    ("OpenBrowser", "Browser with action \"open\""),
    ("BrowserSnapshot", "Browser with action \"snapshot\""),
    ("BrowserClick", "Browser with action \"click\""),
    ("BrowserFill", "Browser with action \"fill\""),
    ("BrowserSelect", "Browser with action \"select\""),
    ("BrowserCheck", "Browser with action \"check\""),
    ("BrowserSetFiles", "Browser with action \"set_files\""),
    ("BrowserWaitFor", "Browser with action \"wait\" (until_text, until_url, until_gone)"),
    ("BrowserHandoff", "Browser with action \"handoff\""),
    ("BrowserRequestBypass", "Browser with action \"request_bypass\""),
    ("BrowserEndBypass", "Browser with action \"end_bypass\""),
    ("BrowserNavigate", "Browser with action \"navigate\""),
    ("BrowserBack", "Browser with action \"back\""),
    ("BrowserForward", "Browser with action \"forward\""),
    ("BrowserReload", "Browser with action \"reload\""),
    ("BrowserEval", "Browser with action \"eval\""),
    ("BrowserDispatchKey", "Browser with action \"key\""),
    ("BrowserFocusElement", "Browser with action \"focus\""),
    ("BrowserFocusInfo", "Browser with action \"focus_info\""),
    ("UIScreenshot", "UI with action \"screenshot\""),
    ("UIQuery", "UI with action \"query\""),
    ("UIClick", "UI with action \"click\""),
    ("ListShortcuts", "UI with action \"shortcuts\""),
    ("RunCommand", "UI with action \"command\""),
    ("PressKeys", "UI with action \"keys\""),
    ("DiscoverWindows", "UI with action \"windows\""),
    ("CaptureWindow", "UI with action \"window\""),
    ("MemoryList", "Memory with action \"list\""),
    ("MemoryRead", "Memory with action \"read\""),
    ("MemoryWrite", "Memory with action \"write\""),
    ("MemoryHistory", "Memory with action \"history\""),
    ("MemoryDiff", "Memory with action \"diff\""),
    ("MemoryRevert", "Memory with action \"revert\" (version_id)"),
    ("GlobalMemoryList", "Memory with action \"list\" and scope \"global\""),
    ("GlobalMemoryRead", "Memory with action \"read\" and scope \"global\""),
    ("GlobalMemoryWrite", "Memory with action \"write\" and scope \"global\""),
    ("GlobalMemoryRemove", "Memory with action \"remove\" and scope \"global\""),
    ("GlobalMemoryHistory", "Memory with action \"history\" and scope \"global\""),
    ("GlobalMemoryDiff", "Memory with action \"diff\" and scope \"global\""),
    ("GlobalMemoryRevert", "Memory with action \"revert\" and scope \"global\""),
    ("WorkEnqueue", "Tasks with action \"enqueue\""),
    ("WorkClaim", "Tasks with action \"claim\""),
    ("WorkHeartbeat", "Tasks with action \"heartbeat\""),
    ("WorkComplete", "Tasks with action \"complete\""),
    ("WorkRelease", "Tasks with action \"release\""),
    ("WorkList", "Tasks with action \"list\""),
    ("WhoIsWorkingOn", "Presence with action \"who\""),
    ("ClaimWork", "Presence with action \"claim\""),
    ("ReleaseWork", "Presence with action \"release\""),
    ("CronCreate", "Schedule with action \"create\" and cron"),
    ("CronList", "Schedule with action \"list\""),
    ("CronPause", "Schedule with action \"pause\""),
    ("CronResume", "Schedule with action \"resume\""),
    ("CronDelete", "Schedule with action \"delete\""),
    ("Loop", "Schedule with action \"create\" and every"),
    ("LoopList", "Schedule with action \"list\""),
    ("LoopStop", "Schedule with action \"delete\""),
    ("WhoAmI", "Pane with action \"whoami\""),
    ("Layout", "Pane with action \"layout\""),
    ("OpenEditor", "Pane with action \"open\" and view \"editor\""),
    ("OpenMedia", "Pane with action \"open\" and view \"media\""),
    ("OpenFiles", "Pane with action \"open\" and view \"files\""),
    ("NewTab", "Pane with action \"new_tab\""),
    ("SetActiveTab", "Pane with action \"activate_tab\""),
    ("FocusWindow", "Pane with action \"focus_window\""),
    ("SetName", "Pane with action \"rename\""),
    ("QuitSelf", "Stop with action \"self\""),
    ("FleetBulkStop", "Stop with action \"agents\""),
    ("ClosePane", "Stop with action \"pane\" (and block_id; to end your own session, action \"self\")"),
    ("WidgetList", "Widget with action \"list\""),
    ("WidgetInstall", "Widget with action \"install\""),
    ("OpenWidget", "Widget with action \"open\""),
    ("PresetList", "Identity with action \"presets\""),
    ("PresetGet", "Identity with action \"preset\""),
    ("IdentityAccounts", "Identity with action \"accounts\""),
    ("IdentityValidate", "Identity with action \"validate\""),
    ("DiscoverAgents", "Agents with action \"list\""),
    ("FleetList", "Agents with action \"list\""),
    ("ListConversations", "Agents with action \"conversations\""),
    ("GetAgentTranscript", "Agents with action \"transcript\""),
    ("OpenAgent", "Agents with action \"open\""),
    ("SearchHistory", "Agents with action \"search_history\""),
    ("SupervisorNudge", "Agents with action \"nudge\" (and decision)"),
    ("SendMessage", "Message with action \"send\""),
    ("FleetBroadcast", "Message with action \"broadcast\""),
    ("PostToChannel", "Message with action \"post\""),
    ("ReadChannel", "Message with action \"read\""),
];

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
        terminal,
        pane_tool,
        stop_tool,
        agents_tool,
        message_tool,
        presence_tool,
        ui_tool,
        browser_tool,
        schedule_tool,
        tasks_tool,
        memory_tool,
        identity_tool,
        widget_tool,
        app_tool
    );
    if let Some((_, instead)) = REPLACED.iter().find(|(old, _)| *old == name) {
        anyhow::bail!("{name} was replaced: use {instead}");
    }
    anyhow::bail!("unknown tool: {name}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    /// Families `call` routes to: each of their tools is advertised.
    const FAMILIES: &[(&str, &str)] = &[
        ("agents_tool", include_str!("agents_tool.rs")),
        ("app_tool", include_str!("app_tool.rs")),
        ("browser_tool", include_str!("browser_tool.rs")),
        ("identity_tool", include_str!("identity_tool.rs")),
        ("memory_tool", include_str!("memory_tool.rs")),
        ("message_tool", include_str!("message_tool.rs")),
        ("pane_tool", include_str!("pane_tool.rs")),
        ("presence_tool", include_str!("presence_tool.rs")),
        ("schedule_tool", include_str!("schedule_tool.rs")),
        ("stop_tool", include_str!("stop_tool.rs")),
        ("tasks_tool", include_str!("tasks_tool.rs")),
        ("terminal", include_str!("terminal.rs")),
        ("ui_tool", include_str!("ui_tool.rs")),
        ("widget_tool", include_str!("widget_tool.rs")),
    ];

    /// Families a family tool calls into, never routed to: their names are
    /// not advertised, and each is in `REPLACED`.
    const INTERNAL: &[(&str, &str)] = &[
        ("accounts", include_str!("accounts.rs")),
        ("browser", include_str!("browser.rs")),
        ("cron", include_str!("cron.rs")),
        ("fleet", include_str!("fleet.rs")),
        ("global_memory", include_str!("global_memory.rs")),
        ("history", include_str!("history.rs")),
        ("loops", include_str!("loops.rs")),
        ("memory", include_str!("memory.rs")),
        ("overlap", include_str!("overlap.rs")),
        ("panes", include_str!("panes.rs")),
        ("presets", include_str!("presets.rs")),
        ("pty_shell", include_str!("pty_shell.rs")),
        ("shell", include_str!("shell.rs")),
        ("ui", include_str!("ui.rs")),
        ("widgets", include_str!("widgets.rs")),
        ("work", include_str!("work.rs")),
    ];

    /// The tool names in a family's `match name` arms (`"A" | "B" => ...`),
    /// read from its `call` function only, so another `match` in the file
    /// (an action, an option) isn't taken for a tool.
    fn arm_names(src: &str) -> Vec<String> {
        let start = src.find("pub(super) async fn call(").expect("a family has a call function");
        let body = &src[start..];
        let body = &body[..body.find("\n}\n").map(|e| e + 2).unwrap_or(body.len())];
        body.lines()
            .filter_map(|l| l.strip_prefix("        \""))
            .filter_map(|l| l.split_once("\" => ").map(|(pat, _)| pat))
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
        for (family, src) in INTERNAL {
            for n in arm_names(src) {
                // Either replaced (not advertised, named in REPLACED) or still
                // advertised and passed through by a routed family.
                if advertised.contains(&n) {
                    assert!(owner.contains_key(&n), "{n} ({family}) is advertised but no routed family passes it through");
                } else {
                    assert!(super::REPLACED.iter().any(|(old, _)| *old == n), "{n} ({family}) has no REPLACED entry");
                }
            }
        }
    }

    #[tokio::test]
    async fn a_replaced_tool_names_its_replacement() {
        let client = reqwest::Client::new();
        let loops = super::LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = super::ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let err = super::call("PtyShellInput", &serde_json::json!({}), &cx).await.unwrap_err();
        assert!(err.to_string().starts_with("PtyShellInput was replaced: use Terminal"), "{err}");
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
