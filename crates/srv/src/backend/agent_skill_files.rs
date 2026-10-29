// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's bound skills, written to its working directory every time its controller starts.
//!
//! `agent.open` and the window's launch write the whole config set (startup file, `.mcp.json`, skill
//! files) when an agent is created. A controller that comes back later (a restart of AgentMux, an
//! upgrade, a resume of a parked agent, a pane that was only reattached) starts from the block as it
//! was, so a skill bound, or made to reach every agent, after that first launch never reached the
//! agent's files until it was launched fresh. Only the skill files are refreshed here: the startup file
//! and `.mcp.json` carry the agent's keys and its own memory, and a resume must not rewrite them.

use std::collections::HashMap;
use std::path::Path;

use crate::backend::agent_config::{
    build_config_files, cleanup_stale_managed_skill_files, managed_skill_file_paths, write_managed_skill_file_manifest,
};
use crate::backend::obj::{meta_get_string, MetaMapType};
use crate::backend::storage::store::Store;

/// The block's agent id: a launch from a definition stamps `agentId`, `agent.open` also `agent:id`.
fn agent_uid_of(meta: &MetaMapType) -> String {
    ["agentId", "agent:id"]
        .iter()
        .map(|key| meta_get_string(meta, key, ""))
        .find(|id| !id.is_empty())
        .unwrap_or_default()
}

/// Write the skill files the agent of a block with this `meta` is bound to, and remove the ones AgentMux
/// wrote earlier that it is no longer bound to. Best-effort: a block that is not an agent's, or whose
/// working directory is gone, is left alone, and a failure is logged, never raised: an agent must start.
/// Returns how many skill files it wrote.
pub fn refresh_for_block(meta: &MetaMapType, mstore: &Store, identity_store: &Store) -> usize {
    let uid = agent_uid_of(meta);
    let work_dir = meta_get_string(meta, "cmd:cwd", "");
    if uid.is_empty() || work_dir.is_empty() {
        return 0;
    }
    let base_path = Path::new(&work_dir);
    if !base_path.is_dir() {
        return 0;
    }
    let Some(agent) = mstore.agent_def_get(&uid).ok().flatten() else {
        return 0;
    };
    let slug = if agent.slug.is_empty() { agent.id.clone() } else { agent.slug.clone() };
    let skills = mstore.effective_skills(identity_store, &agent.id);
    // The same files `agent.open` would write for these skills; only the skill-derived ones are kept.
    let all = build_config_files(&HashMap::new(), &skills, &agent.name, &agent.id, &slug, &work_dir, &agent.provider);
    let wanted = managed_skill_file_paths(all.iter().map(|f| f.filename.as_str()));
    let files: Vec<_> = all.iter().filter(|f| wanted.contains(&f.filename)).collect();

    let Ok(wd) = crate::backend::workdir_fs::Workdir::open(base_path) else {
        tracing::warn!(agent_id = %agent.id, work_dir = %work_dir, "skill files: could not open the working directory");
        return 0;
    };
    cleanup_stale_managed_skill_files(base_path, &wanted);
    let mut written = 0;
    for file in files {
        // Untouched when it already reads the same: the file is read by a CLI that may be about to start.
        if std::fs::read_to_string(base_path.join(&file.filename)).is_ok_and(|now| now == file.content) {
            continue;
        }
        match wd.write(&file.filename, file.content.as_bytes(), false) {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!(agent_id = %agent.id, path = %file.filename, error = %e, "skill files: could not write"),
        }
    }
    write_managed_skill_file_manifest(base_path, &wanted);
    if written > 0 {
        tracing::info!(agent_id = %agent.id, written, "skill files: refreshed for a controller start");
    }
    written
}

#[cfg(test)]
#[path = "agent_skill_files_tests.rs"]
mod tests;
