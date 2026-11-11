// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use crate::backend::storage::store::Store;

/// The files an agent launch writes into its working directory, whichever way it was started: `agent.open`
/// ([`super::agent_open::write_agent_config_files`]) and the window's Launch (`writeagentconfig`, `picker_config::launch_files`)
/// both render them here, so a native and a container agent, and the two entry points, share one builder.
/// `display_name` is what `{{AGENT}}` and `{{AGENT_DISPLAY}}` expand to (a launch's instance name, or the
/// agent's name); `workdir_is_shared` is whether other launches of the agent use the same folder.
pub(crate) fn render_agent_config_files(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    agent: &crate::backend::storage::AgentDefinition,
    display_name: &str,
    agent_slug: &str,
    work_dir: &str,
    workdir_is_shared: bool,
) -> Vec<crate::backend::agent_config::AgentConfigFile> {
    // The agent's content, with Global Memory and its picked bundles put
    // before its own memory (shared with `memory:launchcost`).
    let mut content_map = crate::server::memory_launch_cost::startup_content_map(mstore, id_store, agent);

    // Disable autonomous memory writes only when using the bare slug-fallback workdir
    // (~/.agentmux/agents/<slug>) — that path is shared across same-name multi-tab
    // launches with no collision resolution, risking concurrent MEMORY.md corruption
    // (upstream issue #29051). Any explicitly set workdir, or one a launch allocated, is isolated and
    // keeps writes on.
    if workdir_is_shared {
        disable_autonomous_memory(&mut content_map, work_dir);
    }

    // v1 skills: globals are always injected; the agent's OWN ref-bound skills
    // are authoritative when present, otherwise fall back to legacy
    // db_agent_skills. See Store::effective_skills for the merge algorithm —
    // shared with the `listagentskills` RPC handler so the frontend's
    // pre-launch skill fetch and this materialization path never diverge.
    let effective_skills = mstore.effective_skills(identity_store, &agent.id);

    let mut config_files = crate::backend::agent_config::build_config_files(
        &content_map,
        &effective_skills,
        display_name,
        &agent.id,
        agent_slug,
        work_dir,
        &agent.provider,
    );
    // The files beside a computer-use skill's SKILL.md (its reference files and approved scripts).
    config_files.extend(crate::backend::computer_use_skill::sibling_files(&effective_skills, identity_store));

    // v1 MCP: "agentmux" plus the agent's effective servers. The
    // legacy blob's user servers are merged in ONLY when the agent has no own
    // ref-bound servers (so a global server never wipes a legacy-only agent's
    // .mcp.json). When the agent has own refs, those are authoritative.
    // A bundle carries no MCP servers
    // (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1), so bundles
    // add none.
    let visible_mcp: Vec<crate::backend::storage::McpServer> = mstore.effective_mcp_servers(identity_store, &agent.id, agent.runtime().is_container());
    let has_own_mcp_refs = visible_mcp.iter().any(|server| !server.is_global);
    if !visible_mcp.is_empty() {
        let blob_for_merge = if has_own_mcp_refs {
            None
        } else {
            content_map.get("mcp").map(|s| s.as_str())
        };
        if let Some(mcp_json) = crate::backend::agent_config::build_mcp_config_from_refs(
            &visible_mcp,
            blob_for_merge,
            agent_slug,
            &agent.agent_bus_id,
        ) {
            if let Some(pos) = config_files.iter().position(|f| f.filename == ".mcp.json") {
                config_files[pos].content = mcp_json;
            } else {
                config_files.push(crate::backend::agent_config::AgentConfigFile {
                    filename: ".mcp.json".to_string(),
                    content: mcp_json,
                });
            }
        }
    }

    // Host-tier + LAN-tier jekt sender signing keys
    // (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2,
    // SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1) — inject
    // AGENTMUX_JEKT_KEY / AGENTMUX_LAN_KEY into the agentmux MCP server's
    // own env, right alongside AGENTMUX_AGENT_ID, so the `Message` tool
    // can sign outgoing jekts as this agent. This copy is the fallback: the
    // MCP first fetches its keys from srv (`GET /agentmux/agents/self/keys`).
    // Best-effort: a failure here must never block agent spawn — the agent
    // still launches; only with neither source do its jekts go unsigned.
    // Shared with `WriteAgentConfig`
    // (server/editor_handlers.rs, the "click Launch" path) via
    // `agent_config::inject_jekt_signing_keys_into_mcp_json` — see that
    // function's doc comment for why this must not be duplicated inline
    // again.
    if let Some(file) = config_files.iter_mut().find(|f| f.filename == ".mcp.json") {
        crate::backend::mcp_json_identity::keys_for_agent_open(&mut file.content, &mstore, agent_slug, &agent);
    }
    config_files
}

/// Turn autonomous memory writes off in the agent's settings: the folder is shared by every same-name launch.
fn disable_autonomous_memory(content_map: &mut std::collections::HashMap<String, String>, work_dir: &str) {
    let settings_str = content_map
        .entry("settings".to_string())
        .or_insert_with(|| "{}".to_string());
    match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(settings_str) {
        Ok(mut obj) => {
            obj.entry("autoMemoryEnabled".to_string())
                .or_insert(json!(false));
            *settings_str =
                serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string());
        }
        Err(e) => {
            tracing::warn!(
                work_dir = %work_dir,
                error = %e,
                "write_agent_config_files: settings JSON unparseable; \
                 autoMemoryEnabled guard skipped — memory writes may be active on shared workdir"
            );
        }
    }
}

/// Write one rendered config file into the working directory, the way its kind is written (ownership-aware,
/// exists-guarded, merged, or plainly); a path the workdir refuses is skipped with a warning.
pub(super) fn write_config_file(
    wd: &crate::backend::workdir_fs::Workdir,
    base_path: &std::path::Path,
    expanded_dir: &str,
    file: &crate::backend::agent_config::AgentConfigFile,
) -> Result<(), String> {
    // CLAUDE.md never gets the generic write below — it goes through
    // ownership-aware materialization instead, which never overwrites
    // a pre-existing, non-AgentMux-authored file's content. See
    // docs/specs/SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md.
    if file.filename == "CLAUDE.md" {
        if let Err(e) =
            crate::backend::agent_config::write_claude_md_respecting_ownership(base_path, &file.content)
        {
            return Err(format!("failed to write CLAUDE.md: {e}"));
        }
        return Ok(());
    }
    // Every OTHER provider's startup-instructions filename (AGENTS.md,
    // GEMINI.md, QWEN.md, .pi/APPEND_SYSTEM.md) never overwrites a
    // pre-existing file — codex P1, PR #2788: unlike CLAUDE.md's full
    // ownership-aware materialization above, this is a plain
    // exists-guard (see write_startup_instructions_respecting_existing's
    // own doc comment for why a fuller mechanism isn't attempted here).
    if crate::backend::providers::is_known_startup_instructions_filename(&file.filename) {
        crate::backend::agent_config::write_startup_instructions_respecting_existing(
            base_path,
            &file.filename,
            &file.content,
        )
        .map_err(|e| format!("failed to write {}: {e}", file.filename))?;
        return Ok(());
    }
    // `.mcp.json` never goes into a user's project (#3680): in a folder AgentMux owns it
    // merges into the agent's own file as before; elsewhere it is written under the data
    // directory, outside the project, and Claude is pointed at it at spawn.
    if file.filename == ".mcp.json" {
        if let Some(path) = crate::backend::mcp_config_file::relocated_path(expanded_dir) {
            crate::backend::mcp_config_file::write_relocated(&path, &file.content)
                .map_err(|e| format!("failed to write the agent's MCP config: {e}"))?;
            return Ok(());
        }
        crate::backend::agent_config::write_mcp_json_respecting_user_servers(base_path, &file.content)
            .map_err(|e| format!("failed to write .mcp.json: {e}"))?;
        return Ok(());
    }
    // Through the workdir (SPEC_WORKDIR_SAFE_WRITES_2026_10_01): a path
    // outside it, under a folder linking out of it, or onto a symlink is
    // refused and skipped, like an escaping path always was here.
    match wd.write(&file.filename, file.content.as_bytes(), false) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            tracing::warn!(
                work_dir = %expanded_dir,
                path = %file.filename,
                error = %e,
                "write_agent_config_files: refusing to write a config path outside the working directory"
            );
        }
        other => other.map_err(|e| format!("failed to write {}: {e}", file.filename))?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_config::AgentConfigFile;

    fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    /// #3680: a launch into the user's own project must leave the project's `.mcp.json` exactly
    /// as it was and write no signing material anywhere under the project. The agent's config
    /// goes to AgentMux's data root, where Claude is pointed at it and the other harnesses read it.
    #[test]
    fn a_launch_into_a_user_project_leaves_its_mcp_json_alone_and_writes_no_keys_there() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let project_mcp = project.path().join(".mcp.json");
        let original = "{\n  \"mcpServers\": {\n    \"postgres\": { \"command\": \"pg-mcp\" }\n  },\n  \"kept\": true\n}\n";
        std::fs::write(&project_mcp, original).unwrap();
        let project_str = project.path().to_string_lossy().into_owned();

        let relocated = crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let wd = crate::backend::workdir_fs::Workdir::open(project.path()).unwrap();
            let file = AgentConfigFile {
                filename: ".mcp.json".into(),
                content: r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp","env":{"AGENTMUX_AGENT_ID":"aria","AGENTMUX_JEKT_KEY":"PRIVATE-JEKT","AGENTMUX_LAN_KEY":"PRIVATE-LAN"}}}}"#.into(),
            };
            write_config_file(&wd, project.path(), &project_str, &file).unwrap();
            crate::backend::mcp_config_file::config_path(&project_str)
        });

        assert_eq!(
            std::fs::read_to_string(&project_mcp).unwrap(),
            original,
            "the user's .mcp.json is byte-for-byte unchanged"
        );
        for file in files_under(project.path()) {
            let body = std::fs::read_to_string(&file).unwrap_or_default();
            assert!(
                !body.contains("AGENTMUX_JEKT_KEY") && !body.contains("PRIVATE-JEKT") && !body.contains("PRIVATE-LAN"),
                "no signing material may appear under the project: {file:?}"
            );
        }
        assert!(!project.path().join(".claude").join(".agentmux-managed-mcp-servers.json").exists());
        assert!(relocated.starts_with(data.path().join("mcp")), "{relocated:?}");
        let written = std::fs::read_to_string(&relocated).unwrap();
        assert!(written.contains("agentmux-mcp"), "{written}");
        assert!(written.contains("PRIVATE-JEKT"), "the agent's own config may carry its keys: {written}");
    }

    /// A folder AgentMux owns keeps its own `.mcp.json` (the default agent home), so nothing changes
    /// there and Claude finds it without a flag.
    #[test]
    fn a_launch_into_an_agentmux_folder_keeps_the_file_in_it() {
        let data = tempfile::tempdir().unwrap();
        let home = data.path().join("agents").join("aria");
        std::fs::create_dir_all(&home).unwrap();
        let home_str = home.to_string_lossy().into_owned();
        let relocated = crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let wd = crate::backend::workdir_fs::Workdir::open(&home).unwrap();
            let file = AgentConfigFile {
                filename: ".mcp.json".into(),
                content: r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#.into(),
            };
            write_config_file(&wd, &home, &home_str, &file).unwrap();
            crate::backend::mcp_config_file::relocated_path(&home_str)
        });
        assert!(home.join(".mcp.json").is_file());
        assert!(relocated.is_none());
    }
}
