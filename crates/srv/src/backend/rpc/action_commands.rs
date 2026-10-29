// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which websocket RPC commands are recorded ([`super::action_record`]), decided one
//! by one. Every command a connection can call is in exactly one of the lists below
//! (a test checks it against the commands a real connection registers), so a new
//! command cannot be added without someone deciding whether it changes anything.

/// Whether a call of a command is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Every call, whoever makes it.
    Always,
    /// Calls by an agent or an unproven connection. The window's own view state
    /// (layout, tabs, drafts, notifications) is not worth a line each time the
    /// operator moves something; a call to it from anywhere else is.
    UnlessOperator,
    /// Never: it reads, or it is live input or a subscription.
    Never,
}

/// Changes state, runs or stops a process, writes files, sends to an agent or
/// changes who may do what. Recorded whoever calls.
pub const ALWAYS: &[&str] = &[
    "account.key.verify", "account.oauth.cancel", "account.oauth.client.clear", "account.oauth.start", "agent.account.switch",
    "agent.define", "agent.kill-process", "agent.kill-tree", "agent.move-here", "agent.open", "agent.pane.unregister", "agent.send",
    "agent.setSeat", "agent.start", "agent.stop", "agent.takeover", "agent:memory:revert", "agent:memory:write_file",
    "agent:session:append_output", "agent:session:archive", "agent:session:write_state", "agentanswer", "agentcancel",
    "agentdefcreatefromtemplate", "agentdefhide", "agentdefunhide", "agentinput", "agentlastruntime",
    "agents.migrateForkSlugs", "agentshutdownkeep", "agentshutdownnow", "agentstop", "app.desktop", "app.upgrade-requests",
    "appendagenthistory", "asksidequestion", "attachments.copy-to-workdir", "attachments.ingest", "auth.cancel",
    "auth.start", "auth.submitcallback", "browser_profiles.create", "browser_profiles.delete",
    "browser_profiles.update", "browser_start_page.set", "bundle.delete", "bundle.import", "bundle.import.commit",
    "bundle.import_for_agent", "bundle.upsert", "connconnect", "conndisconnect", "connensure", "connsessionend",
    "copyforkhistory", "createagent", "createagentinstance", "createagentskill", "createeditordir",
    "createeditorfile", "createscratchfile", "deleteagent", "deleteagentinstance", "deleteagentskill", "deletedrone",
    "deleteeditorfile", "deleteidentityaccount", "deletememory", "deletesystemmemory", "fleet.broadcast",
    "fleet.bulk-stop", "fleet.group.create", "fleet.group.delete", "fleet.group.update", "forkagentdefinition",
    "fs.create", "fs.delete", "fs.op.cancel", "fs.op.resolve", "fs.op.start", "fs.open", "fs.rename", "fs.restore",
    "fs.trash", "github.connection", "globalmemory:import", "globalmemory:revert", "hidenamedagent", "history.reclaim",
    "identity.account.move_agents", "identity.account.upsert", "identity.ensureaccountdir", "identity.self.unlink",
    "importagentfromclaw", "importagents", "install.cancel", "install.start", "installtool", "jobs.act", "jobs.save",
    "jobs.templates.approve", "jobs.templates.catalog.install", "jobs.templates.install", "jobs.templates.uninstall",
    "kanban.act", "linkagentidentity", "lspstart", "lspstop", "mcp.bind", "mcp.catalog.bind", "mcp.catalog.delete",
    "mcp.catalog.probe", "mcp.catalog.set_reach", "mcp.catalog.unbind", "mcp.catalog.upsert", "mcp.delete",
    "mcp.probe", "mcp.unbind", "mcp.upsert", "memory.write", "movescratchfile", "muxbus.cloudconfig",
    "muxbus.disconnect", "muxbus.login", "muxbus.login.cancel", "openinshell", "pins.adopt", "pins.forget",
    "provider.ensureauthdir", "remoteadd", "remoteagentrevoke", "remoteforget", "remotehelperremove",
    "remotesetconfig", "remotetest", "renameagentdefinitiontitle", "renameeditorfile", "reorderglobalbrain",
    "reseedagents", "routeannounce", "routeunannounce", "runclilogin", "rundrone", "sensitive.decide", "session:archive",
    "session:restore", "setagentbundles", "setagentcontent", "setconfig", "shellexec", "shellstop", "skill.bind",
    "skill.catalog.bind", "skill.catalog.bind_all", "skill.catalog.bind_to_bundle", "skill.catalog.delete",
    "skill.catalog.set_reach", "skill.catalog.unbind",
    "skill.catalog.unbind_from_bundle", "skill.catalog.upsert", "skill.catalog.upsert_for_bundle", "skill.delete",
    "skill.unbind", "skill.upsert", "subprocessspawn", "supervisor.set-mode", "toolchain.install_system_tool",
    "toolchain.prune", "tooldecision", "tower.forget", "tower.pair", "unlinkagentidentity", "updateagent",
    "updateagentinstance", "updateagentskill", "upsertdrone", "upsertidentityaccount", "upsertmemory",
    "upsertsystemmemory", "viewer.pair-start", "viewer.revoke", "widget.api", "widgets.call",
    "widgets.catalog.install", "widgets.install", "widgets.rescan", "widgets.session", "widgets.setenabled",
    "widgets.uninstall", "writeagentconfig", "writeeditorfile",
    "updatecontainer",
    "identity.account.signed_in", "identity.account.validate",
];

/// The window's own view and bookkeeping state.
pub const UNLESS_OPERATOR: &[&str] = &[
    "agent.input-draft", "agent.input-history-append", "agent.input-history-clear", "agent.input-history-delete",
    "agent.input-history-restore", "agent.input-history-trash", "ambientnarrate", "attachments.cancel",
    "backgroundtaskcompletion", "backgroundtaskpid", "block.reveal", "blockfile:write_state", "bookmarks.set",
    "controllerresync", "createsubblock", "deletesubblock", "docknodestatus", "fs.reveal", "fs.unwatch", "fs.watch",
    "layout.applyIntent", "layout.open", "layout.restoreOtherBuild", "layout.save", "memorydelivery:claim_fallback",
    "notify.ack", "notify.attention", "notify.emit", "notify.focus", "notify.takeactivation", "notify.test",
    "operator.challenge", "pane.activateTab", "pane.moveTab", "pane.moveTabs", "pane.open", "pane.removeTab",
    "presence.publish-now", "setmeta", "unwatcheditorfile", "unwatchmediadir", "viewer.agent-hidden",
    "watcheditorfile", "watchmediadir", "widgets.endsession",
];

/// Terminal and language-server input and event subscriptions: live traffic, not actions.
pub const LIVE_INPUT: &[&str] = &[
    "agent:open_tail", "controllerinput", "eventsub", "eventunsub", "eventunsuball", "lspsend",
];

/// Returns data and changes nothing.
pub const READ_ONLY: &[&str] = &[
    "account.oauth.client.get", "account.oauth.poll", "agent.input-history",
    "agent.input-history-search", "agent.list", "agent.open-panes", "agent.output", "agent.process-list",
    "agent.project_instructions", "agent.status", "agent.tracked-blocks", "agent:memory:adoption_list",
    "agent:memory:claims", "agent:memory:diff", "agent:memory:history", "agent:memory:list", "agent:memory:read_file",
    "agent:session:list_archives", "agent:session:read", "agentapiretry", "agentdeflisthiddentemplates", "agentruntime.list",
    "ambient.outcomes", "app.update-check", "app.update-status", "attachments.info", "auth.poll",
    "blockfile:line_count", "blockfile:read_range", "blockfile:read_state", "blockstats.detail", "bookmarks.list",
    "browser_profiles.list", "bundle.export", "bundle.export_for_agent", "bundle.export_for_agent_with_history",
    "bundle.get", "bundle.import.preview", "bundle.list", "bundle.self.get", "bundle.validate", "checkcliauth",
    "connectivity.status", "connlist", "connlistaws", "connsessions", "containerimagecheck",
    "containerruntimeavailable", "eventreadhistory", "exportagents", "fileprobe", "fleet.group.list",
    "forkagentdefinitionsuggest", "fs.git_status", "fs.list", "fs.places", "getagentbundles", "getagentcontent",
    "getagentinstance", "getallagentcontent", "getbuildslot", "getclaudeglobalconfig", "getdrone", "geteditorhome",
    "geteditorroots", "getfullconfig", "getidentityaccount", "getmemory", "getmeta", "gettoolstatus",
    "getwaveairatelimit", "globalmemory:diff", "globalmemory:history", "globalmemory:import_sources",
    "globalmemory:removed", "globalmemory:sections", "history.agent_sessions", "history.agents", "history.locate",
    "history.usage", "identity.account.availability",
    "identity.self.accounts", "install.check", "jobs.dryrun", "jobs.get", "jobs.list", "jobs.runs",
    "jobs.templates.catalog", "jobs.templates.get", "jobs.templates.list", "jobs.templates.update_diff",
    "kanban.list", "kanban.task", "layout.otherBuilds", "layout.preview", "listagenthistory", "listagentidentities",
    "listagentinstances", "listagents", "listagentskills", "listallagentidentities", "listbackgroundtasks",
    "listdroneruns", "listdrones", "listeditordir", "listidentityaccounts", "listmemories", "listnamedagents",
    "listrecentsessions", "mcp.catalog.list", "mcp.catalog.list_for_agent", "mcp.get", "mcp.list",
    "mcp.observed.list", "memory.list", "memory.read", "memory:launchcost", "memorydelivery:compose", "muxbus.status",
    "pins.list", "presence.status", "providers.models", "reactive.registration_flags", "reactive.registrations", "readeditorfile", "remoteslist",
    "remotesshlocate", "resolve.prereqs", "resolvecli", "searchagenthistory", "session:activity_summary",
    "session:export", "session:next_prompt_suggestion", "session:resume_preflight", "shell.list", "shellstatus",
    "skill.catalog.list", "skill.catalog.list_for_agent", "skill.catalog.list_for_bundle", "skill.get", "skill.list",
    "swarm.other-instances", "toolchain.env", "toolchain.resolve_install_command", "toolchain.versions",
    "tower.peers", "tower.sample", "viewer.can-send", "viewer.devices", "voice.checkPath", "waveinfo",
    "widget.health", "widgets.catalog", "widgets.list", "widgets.readfile", "widgets.requests",
    "widgets.sectionentry", "wsllist",
];

/// How `command` is recorded; `None` for a command in no list.
pub fn policy(command: &str) -> Option<Policy> {
    if ALWAYS.contains(&command) {
        Some(Policy::Always)
    } else if UNLESS_OPERATOR.contains(&command) {
        Some(Policy::UnlessOperator)
    } else if LIVE_INPUT.contains(&command) || READ_ONLY.contains(&command) {
        Some(Policy::Never)
    } else {
        None
    }
}

/// Commands that only read unless the request carries the named field.
const WRITES_ONLY_WITH: &[(&str, &str)] = &[("agentlastruntime", "runtime")];

/// Whether this request, of a command that can write, only reads.
pub fn is_a_read(command: &str, data: &serde_json::Value) -> bool {
    WRITES_ONLY_WITH
        .iter()
        .any(|(c, field)| *c == command && data.get(*field).map_or(true, serde_json::Value::is_null))
}

/// A command that is in no list is recorded: forgetting to decide never hides a call.
pub fn is_recorded(command: &str, operator: bool) -> bool {
    match policy(command) {
        Some(Policy::Never) => false,
        Some(Policy::UnlessOperator) => !operator,
        Some(Policy::Always) | None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_in_one_list_only() {
        let lists: [(&str, &[&str]); 4] =
            [("ALWAYS", ALWAYS), ("UNLESS_OPERATOR", UNLESS_OPERATOR), ("LIVE_INPUT", LIVE_INPUT), ("READ_ONLY", READ_ONLY)];
        let mut seen = std::collections::HashMap::new();
        for (list, names) in lists {
            for name in names {
                if let Some(other) = seen.insert(*name, list) {
                    panic!("{name} is in {other} and {list}");
                }
            }
        }
    }

    #[test]
    fn the_policies() {
        assert!(is_recorded("agent.kill-tree", false) && is_recorded("agent.kill-tree", true));
        assert!(is_recorded("setmeta", false) && !is_recorded("setmeta", true));
        assert!(!is_recorded("listagents", false) && !is_recorded("controllerinput", false));
        assert!(is_recorded("a-command-nobody-decided", true), "an undecided command is recorded");
        let read = serde_json::json!({"id": "a"});
        let write = serde_json::json!({"id": "a", "runtime": ""});
        assert!(is_a_read("agentlastruntime", &read) && !is_a_read("agentlastruntime", &write));
        assert!(!is_a_read("agent.send", &read));
    }
}
