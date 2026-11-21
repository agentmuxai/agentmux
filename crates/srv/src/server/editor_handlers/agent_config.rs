// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `writeagentconfig`, moved out of `register_editor_handlers` unchanged as named steps: write an agent's
//! config files atomically into its working directory.

use std::path::Path;
use std::sync::Arc;

use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc_types::{AgentConfigFile, CommandWriteAgentConfigData, CommandWriteAgentConfigResult, RpcContext};
use crate::backend::storage::store::Store;

/// The stores a config write reads (the agent's picked bundles, its identity links, the pane store).
pub(super) struct ConfigStores {
    pub id_store: Arc<Store>,
    pub mstore: Arc<Store>,
    pub identity_store: Arc<Store>,
}

/// Only the window, proven by its host, gets an agent's keys (`hands_out_keys`); an agent writes only its own
/// (`agent_config_guard`).
pub(super) fn write_agent_config(
    stores: &ConfigStores,
    ctx: &RpcContext,
    hands_out_keys: bool,
    mut cmd: CommandWriteAgentConfigData,
) -> Result<CommandWriteAgentConfigResult, String> {
    let ConfigStores { id_store, mstore, identity_store } = stores;
    crate::server::agent_config_guard::check_agent_config_write(
        mstore,
        ctx,
        cmd.agent_id.as_deref(),
        &expand_home_dir_safe(&cmd.working_dir),
        cmd.auto_allocate,
    )?;
    tracing::info!(
        working_dir = %cmd.working_dir,
        file_count = cmd.files.len(),
        auto_allocate = cmd.auto_allocate,
        agent_type = cmd.agent_type.as_deref().unwrap_or("host"),
        "WriteAgentConfig"
    );
    let agent_mode = cmd.agent_type.clone().unwrap_or_else(|| "host".to_string());
    crate::server::picker_config::check_render_request(mstore, &cmd, hands_out_keys)?;
    let final_working_dir = resolve_workdir(&cmd)?;
    cmd.files = crate::server::picker_config::files_to_write(
        std::mem::take(&mut cmd.files),
        mstore,
        id_store,
        identity_store,
        &cmd,
        &agent_mode,
        hands_out_keys,
        &final_working_dir,
    )?;
    let on_host = !crate::backend::runtime_adapter::RuntimeKind::of_agent_type(&agent_mode).is_container();
    write_files(&cmd.files, Path::new(&final_working_dir), on_host)?;
    // The final path goes back so the caller can patch `cmd:cwd` if collision resolution changed it.
    Ok(CommandWriteAgentConfigResult { working_dir: final_working_dir })
}

/// The final on-disk path. An auto-generated instance path (`auto_allocate: true`) uses the atomic `<base>-N`
/// allocator so concurrent same-hour launches don't share a workdir. A user-specified path is mkdir -p'd and
/// never rewritten.
fn resolve_workdir(cmd: &CommandWriteAgentConfigData) -> Result<String, String> {
    let expanded_working_dir = expand_home_dir_safe(&cmd.working_dir);
    if cmd.auto_allocate {
        let desired = expanded_working_dir.to_string_lossy().to_string();
        return crate::server::app_api::allocate_agent_workdir(&desired);
    }
    let p = expanded_working_dir.as_path();
    if !p.exists() {
        std::fs::create_dir_all(p) // workdir-fs: creates the workdir itself
            .map_err(|e| format!("failed to create working dir: {e}"))?;
    }
    Ok(expanded_working_dir.to_string_lossy().to_string())
}

fn write_files(files: &[AgentConfigFile], base_path: &Path, on_host: bool) -> Result<(), String> {
    // A host agent in a folder a container agent can write is given its files from outside the folder
    // (`host_agent_files`), as `agent.open` gives them: nothing is written into the folder.
    if on_host && crate::backend::host_agent_files::write(&base_path.to_string_lossy(), files.iter().map(|f| (f.path.as_str(), f.content.as_str())))? {
        return Ok(());
    }
    // The base is canonicalized ONCE (it exists: allocate_agent_workdir or the explicit-path mkdir -p created
    // it). The per-file symlink-escape verifier uses it to catch a symlinked ancestor like
    // `<base>/.claude -> /tmp/outside` before fs::write follows it.
    let canonical_base = base_path
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize working dir {}: {e}", base_path.display()))?;

    // Remove skill-derived files (.claude/commands/*.md, .claude/skills/*/SKILL.md) that WE wrote on a
    // previous materialization but aren't part of this run's output, e.g. a skill's format switched between
    // "prompt" and "agent-skill", or a skill was renamed/removed. This is the "click Launch" path used on every
    // normal agent launch (`launchAgentDefinition`/`WriteAgentConfigCommand`), shared with `agent.open`'s
    // `write_agent_config_files` (agent_open.rs) via `agent_config.rs` so the two can't drift apart (#2322).
    let wd = crate::backend::workdir_fs::Workdir::open(base_path)
        .map_err(|e| format!("failed to open working dir {}: {e}", base_path.display()))?;
    let new_managed_skill_paths = crate::backend::agent_config::managed_skill_file_paths(files.iter().map(|f| f.path.as_str()));
    crate::backend::agent_config::cleanup_stale_managed_skill_files(base_path, &new_managed_skill_paths);

    for file in files {
        write_file(&wd, base_path, &canonical_base, file, on_host)?;
    }
    crate::backend::agent_config::write_managed_skill_file_manifest(base_path, &new_managed_skill_paths);
    Ok(())
}

/// One config file, written the way its kind needs: CLAUDE.md and the other providers' startup files keep an
/// existing file's content, `.mcp.json` merges into the user's own, anything else is an atomic workdir write.
fn write_file(wd: &crate::backend::workdir_fs::Workdir, base_path: &Path, canonical_base: &Path, file: &AgentConfigFile, on_host: bool) -> Result<(), String> {
    // A lexical join plus traversal check works on Windows where canonicalize() adds the `\\?\` UNC prefix and
    // breaks starts_with against not-yet-created files. It catches `..`, absolute paths and drive-letter
    // prefixes (root and inner).
    let file_path = crate::backend::base::safe_join_within_base(base_path, &file.path)
        .map_err(|e| format!("path traversal denied: {} ({e})", file.path))?;
    // The symlink-escape guard: if any EXISTING ancestor is a symlink that resolves outside the workdir, reject.
    // A no-op for fully fresh agent dirs. Like a refused write below, this skips that one file rather than
    // failing the launch, as agent.open does (#4141).
    if let Err(e) = crate::backend::base::verify_no_symlink_escape(&file_path, canonical_base) {
        tracing::warn!(path = %file.path, error = %e, "writeagentconfig: not writing a config file under a folder linking outside the workdir");
        return Ok(());
    }
    // Global Memory is already in `file.content` (`picker_config`), so launching from the picker gives the same
    // workspace rules as the agent.open RPC path.
    if file.path == "CLAUDE.md" {
        // Ownership-aware materialization instead of the generic write below: never overwrites a pre-existing,
        // non-AgentMux-authored file's content. See docs/specs/SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md.
        crate::backend::agent_config::write_claude_md_respecting_ownership(base_path, &file.content)
            .map_err(|e| format!("failed to write CLAUDE.md: {e}"))?;
        tracing::debug!(path = %file_path.display(), "wrote config file (ownership-aware)");
        return Ok(());
    }
    // This RPC command carries no provider ID (`CommandWriteAgentConfigData` is just `{working_dir, files}`:
    // the frontend already resolved everything), so a non-Claude provider's startup-instructions file
    // (AGENTS.md, GEMINI.md, QWEN.md, .pi/APPEND_SYSTEM.md, ...) is recognized by filename membership against
    // the provider registry, not a literal string match.
    if crate::backend::providers::is_known_startup_instructions_filename(&file.path) {
        // Never overwrites a pre-existing file (#2788): a simpler exists-guard than CLAUDE.md's full
        // ownership-aware materialization. See write_startup_instructions_respecting_existing for why a fuller
        // mechanism (the @import-based side-file fallback) isn't attempted here.
        crate::backend::agent_config::write_startup_instructions_respecting_existing(base_path, &file.path, &file.content)
            .map_err(|e| format!("failed to write {}: {e}", file.path))?;
        tracing::debug!(path = %file_path.display(), "wrote config file (exists-guarded)");
        return Ok(());
    }
    // `.mcp.json` never goes into a user's project (#3680): in a folder AgentMux owns it merges into the
    // agent's own file; elsewhere it is written under the data directory, outside the project. A host
    // agent's also goes outside a folder a container agent can write, with nothing of that folder's file.
    if file.path == ".mcp.json" {
        let expanded = base_path.to_string_lossy();
        let outside = if on_host {
            crate::backend::mcp_config_file::host_relocated_path(&expanded)
        } else {
            crate::backend::mcp_config_file::relocated_path(&expanded)
        };
        if let Some(path) = outside {
            let content = if on_host { crate::backend::mcp_config_file::host_content(&expanded, file.content.clone()) } else { file.content.clone() };
            crate::backend::mcp_config_file::write_relocated(&path, &content)
                .map_err(|e| format!("failed to write the agent's MCP config: {e}"))?;
            tracing::debug!(path = %path.display(), "wrote config file (relocated, owner-only)");
            return Ok(());
        }
        crate::backend::agent_config::write_mcp_json_respecting_user_servers(base_path, &file.content)
            .map_err(|e| format!("failed to write .mcp.json: {e}"))?;
        tracing::debug!(path = %file_path.display(), "wrote config file (merged, owner-only)");
        return Ok(());
    }
    // Atomic, parents created inside the workdir, never through a symlinked file
    // (SPEC_WORKDIR_SAFE_WRITES_2026_10_01). A refusal skips that one file, as agent.open does (#4141); any
    // other failure fails the launch.
    match wd.write(&file.path, file.content.as_bytes(), false) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            tracing::warn!(path = %file.path, error = %e, "writeagentconfig: not writing a config file");
        }
        other => {
            other.map_err(|e| format!("failed to write {}: {e}", file.path))?;
            tracing::debug!(path = %file_path.display(), "wrote config file");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<AgentConfigFile> {
        [("CLAUDE.md", "be kind"), (".claude/settings.json", "{\"hooks\":{}}"), (".claude/skills/review/SKILL.md", "body"), (".mcp.json", "{\"mcpServers\":{}}")]
            .into_iter()
            .map(|(path, content)| AgentConfigFile { path: path.to_string(), content: content.to_string() })
            .collect()
    }

    /// The window's Launch, for a host agent in a folder a container agent has run in: every file goes to the
    /// agent's own folder outside it, as `agent.open` writes them, and nothing into the folder. The container
    /// agent's own launch there, and a host launch into any other folder, write into the folder as before.
    #[test]
    fn a_host_launch_into_a_container_agents_folder_writes_outside_it() {
        let (data, folder, plain) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        let in_folder = |root: &Path, name: &str| root.join(name).exists();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            crate::backend::mcp_folder_rule::mark(&dir);
            write_files(&files(), folder.path(), true).unwrap();
            assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none(), "nothing is written into the folder");
            let own = crate::backend::host_agent_files::dir_of(&dir).unwrap();
            assert!(crate::backend::host_agent_files::settings_path(&own).is_file());
            assert!(crate::backend::host_agent_files::instructions_path(&own).is_file());
            assert!(crate::backend::host_agent_files::plugin_dir(&own).join("skills").join("review").join("SKILL.md").is_file());

            write_files(&files(), folder.path(), false).unwrap();
            assert!(in_folder(folder.path(), "CLAUDE.md") && in_folder(folder.path(), ".claude/settings.json"), "the container agent's own launch writes its folder");

            write_files(&files(), plain.path(), true).unwrap();
            assert!(in_folder(plain.path(), "CLAUDE.md") && in_folder(plain.path(), ".claude/settings.json") && in_folder(plain.path(), ".claude/skills/review/SKILL.md"), "a folder no container agent has to do with: as before");
            assert_eq!(crate::backend::host_agent_files::dir_of(&plain.path().to_string_lossy()), None);
        });
    }
}
