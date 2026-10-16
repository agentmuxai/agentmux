// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `memory:launchcost`: what an agent's next launch delivers into its context,
//! part by part, in estimated tokens (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md
//! §4.2). A dry run of the launch's own composition — the same functions,
//! nothing written — so its total is the estimate of the startup file a launch
//! writes, plus what Claude Code's SessionStart hook and CLI add.

use std::collections::HashMap;
use std::sync::Arc;

use agentmux_common::tokens::{count_lines, estimate_tokens};

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{CommandMemoryLaunchCostData, LaunchCost, LaunchCostPart, COMMAND_MEMORY_LAUNCH_COST};
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;

use super::AppState;

/// The agent's content by type (`soul`, `agentmd`, `memory`, …), with Global
/// Memory for its kind and its picked bundles put before its own `memory`, as
/// the startup file carries them. Shared by the launch
/// (`app_api::agent_open::write_agent_config_files`) and `memory:launchcost`.
pub(crate) fn startup_content_map(mstore: &Store, id_store: &Store, agent: &AgentDefinition) -> HashMap<String, String> {
    let mut content_map: HashMap<String, String> = mstore
        .agent_content_get_all(&agent.id)
        .unwrap_or_default()
        .into_iter()
        .map(|fc| (fc.content_type, fc.content))
        .collect();
    let startup_block = startup_block(mstore, id_store, agent);
    if !startup_block.is_empty() {
        content_map
            .entry("memory".to_string())
            .and_modify(|existing| *existing = format!("{startup_block}\n\n---\n\n{existing}"))
            .or_insert(startup_block);
    }
    content_map
}

/// Global Memory (minus Operator Config for another agent kind), then the
/// agent's picked bundles in list order (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6).
fn startup_block(mstore: &Store, id_store: &Store, agent: &AgentDefinition) -> String {
    let global = global_bundles(id_store, agent);
    crate::backend::storage::join_startup_blocks(
        &crate::backend::storage::format_global_bundle_block(&global),
        &crate::backend::storage::format_agent_bundle_block(&crate::backend::storage::agent_picked_bundles(
            mstore, id_store, &agent.id,
        )),
    )
}

fn global_bundles(id_store: &Store, agent: &AgentDefinition) -> Vec<crate::backend::storage::store::Bundle> {
    crate::backend::operator_config_seed::global_bundles_for_agent(id_store.bundle_list_global().unwrap_or_default(), &agent.agent_type)
}

/// Whether AgentMux's startup text reaches the agent by file. A user-owned
/// file isn't overwritten; Claude still gets the text through an `@import` of
/// `.claude/AGENTMUX_MEMORY.md`.
fn delivered_by_file(has_startup_file: bool, is_claude: bool, on_disk: Option<&str>) -> bool {
    has_startup_file && (is_claude || on_disk.is_none_or(crate::backend::agent_config::is_agentmux_managed_instructions))
}

fn part(kind: &str, id: Option<&str>, name: &str, text: &str, via: &str) -> LaunchCostPart {
    LaunchCostPart {
        kind: kind.to_string(),
        id: id.map(str::to_string),
        name: name.to_string(),
        tokens_est: estimate_tokens(text),
        size_bytes: text.len() as u64,
        lines: count_lines(text),
        via: via.to_string(),
    }
}

/// `agent`'s launch cost. `startup_file_on_disk` is the agent's existing
/// startup file, if any, to tell whether AgentMux owns it; `personal` is the
/// agent's Personal Memory files `(filename, content)` (Claude only).
pub(crate) fn launch_cost(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    agent: &AgentDefinition,
    startup_file_on_disk: Option<&str>,
    personal: &[(String, String)],
) -> LaunchCost {
    let provider = crate::backend::providers::get_provider(&agent.provider);
    let provider_id = provider.map_or(agent.provider.as_str(), |p| p.id);
    let is_claude = provider_id == "claude";
    let startup_file = provider.and_then(|p| p.startup_instructions_filename).map(str::to_string);
    let delivered_by_file = delivered_by_file(startup_file.is_some(), is_claude, startup_file_on_disk);

    let mut parts = Vec::new();
    if startup_file.is_some() {
        const VIA: &str = "startup_file";
        for s in crate::backend::storage::global_bundle_sections(&global_bundles(id_store, agent)) {
            let kind = if s.is_system { "operator_config" } else { "workspace_global" };
            parts.push(part(kind, Some(&s.id), &s.name, &s.text, VIA));
        }
        for b in crate::backend::storage::agent_picked_bundles(mstore, id_store, &agent.id) {
            let text = crate::backend::storage::format_agent_bundle_block(std::slice::from_ref(&b));
            if !text.is_empty() {
                parts.push(part("bundle", Some(&b.id), &b.name, &text, VIA));
            }
        }
        let own: HashMap<String, String> = mstore
            .agent_content_get_all(&agent.id)
            .unwrap_or_default()
            .into_iter()
            .map(|fc| (fc.content_type, fc.content))
            .collect();
        for (key, kind, name) in [("soul", "soul", "Soul"), ("agentmd", "agentmd", "Agent instructions"), ("memory", "agent_memory", "Agent memory")] {
            if let Some(text) = own.get(key).filter(|t| !t.trim().is_empty()) {
                parts.push(part(kind, None, name, text, VIA));
            }
        }
        let skills = mstore.effective_skills(identity_store, &agent.id);
        let index = crate::backend::agent_config::skills_index_text(&skills, provider_id);
        if !index.is_empty() {
            parts.push(part("skills_index", None, "Skills index", &index, VIA));
        }
        // What the parts don't count — the `# Memory` heading, separators and
        // template expansion — so the total is the file's own estimate.
        let file = file_text(mstore, id_store, identity_store, agent, provider_id);
        let file_tokens = estimate_tokens(&file);
        let counted: u64 = parts.iter().map(|p| p.tokens_est).sum();
        if file_tokens > counted {
            parts.push(LaunchCostPart {
                kind: "formatting".to_string(),
                id: None,
                name: "Headings and separators".to_string(),
                tokens_est: file_tokens - counted,
                size_bytes: 0,
                lines: 0,
                via: VIA.to_string(),
            });
        }
    }
    if is_claude {
        // The SessionStart hook delivers every topic file; the CLI loads
        // MEMORY.md itself.
        for (name, text) in personal {
            let via = if name == "MEMORY.md" { "cli" } else { "session_hook" };
            parts.push(part("personal", None, name, text, via));
        }
    }
    let total_tokens_est = parts.iter().map(|p| p.tokens_est).sum();
    LaunchCost {
        agent_id: agent.id.clone(),
        provider: provider_id.to_string(),
        startup_file,
        delivered_by_file,
        parts,
        total_tokens_est,
    }
}

/// The startup file a launch would write for `agent`.
fn file_text(mstore: &Store, id_store: &Store, identity_store: &Store, agent: &AgentDefinition, provider_id: &str) -> String {
    let content_map = startup_content_map(mstore, id_store, agent);
    let skills = mstore.effective_skills(identity_store, &agent.id);
    let work_dir = work_dir(agent);
    let filename = crate::backend::providers::get_provider(provider_id).and_then(|p| p.startup_instructions_filename);
    crate::backend::agent_config::build_config_files(&content_map, &skills, &agent.name, &agent.id, &agent.slug, &work_dir, provider_id)
        .into_iter()
        .find(|f| Some(f.filename.as_str()) == filename)
        .map(|f| f.content)
        .unwrap_or_default()
}

/// The folder the agent launches in, as `agent.open` picks it.
fn work_dir(agent: &AgentDefinition) -> String {
    if agent.working_directory.is_empty() {
        crate::backend::storage::agents::default_agent_working_dir(&agent.name)
    } else {
        agent.working_directory.clone()
    }
}

/// Reads `*.md` in the agent's Personal Memory folder, capped like every
/// memory read.
fn personal_files(mstore: &Store, agent: &AgentDefinition) -> Vec<(String, String)> {
    use std::io::Read;
    let Some(dir) = super::native_memory_handlers::memory_dir_for_agent_by_id(mstore, agent) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<(String, String)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".md") {
                return None;
            }
            let mut buf = Vec::new();
            std::fs::File::open(e.path())
                .and_then(|f| f.take(super::native_memory_handlers::MAX_MEMORY_FILE_BYTES).read_to_end(&mut buf))
                .ok()?;
            Some((name, String::from_utf8_lossy(&buf).into_owned()))
        })
        .collect();
    files.sort();
    files
}

pub(crate) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_MEMORY_LAUNCH_COST, move |cmd: CommandMemoryLaunchCostData, _ctx| {
        let (mstore, id_store, identity_store) = (mstore.clone(), id_store.clone(), identity_store.clone());
        async move {
            let agents = mstore.agent_def_list().map_err(|e| format!("memory:launchcost: {e}"))?;
            let agent = agents
                .into_iter()
                .find(|a| a.id == cmd.agent_id)
                .ok_or_else(|| format!("memory:launchcost: no agent {}", cmd.agent_id))?;
            let provider = crate::backend::providers::get_provider(&agent.provider);
            let on_disk = provider
                .and_then(|p| p.startup_instructions_filename)
                .and_then(|f| std::fs::read_to_string(crate::backend::base::expand_home_dir_safe(&work_dir(&agent)).join(f)).ok());
            let personal = if provider.is_some_and(|p| p.id == "claude") { personal_files(&mstore, &agent) } else { Vec::new() };
            Ok::<LaunchCost, String>(launch_cost(&mstore, &id_store, &identity_store, &agent, on_disk.as_deref(), &personal))
        }
    });
}

#[cfg(test)]
mod tests {
    use super::delivered_by_file;

    #[test]
    fn a_user_owned_startup_file_stops_delivery_except_for_claude() {
        let managed = "<!-- agentmux:managed-startup-instructions -->\n\n# Memory\n";
        assert!(delivered_by_file(true, false, None), "no file yet: the launch writes it");
        assert!(delivered_by_file(true, false, Some(managed)));
        assert!(!delivered_by_file(true, false, Some("# My own AGENTS.md\n")));
        assert!(delivered_by_file(true, true, Some("# My own CLAUDE.md\n")), "Claude: through @import");
        assert!(!delivered_by_file(false, false, None), "no startup file (Kimi)");
    }
}
