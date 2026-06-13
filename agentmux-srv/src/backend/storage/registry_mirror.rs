// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cross-version named-agent registry mirror.
//!
//! Mirrors `db_agent_instances` mutations into the JSON registry at
//! `~/.agentmux/agents/registry/` so other AgentMux versions running
//! on the same machine see each other's named agents in their launch
//! modal dropdowns. SQLite is authoritative for the local version;
//! the registry is a best-effort cross-version view.
//!
//! Extracted from `store.rs` in Phase R.6 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). This whole
//! file retires when Phase R sunsets the JSON registry entirely;
//! isolating it now makes that future deletion a clean unit.

use std::path::Path;

use super::store::{AgentInstance, Store};

/// Build a registry record from an `AgentInstance`. Returns an error
/// if the working directory can't be expressed as a path relative to
/// the canonical shared agents root (e.g. user pointed an agent at
/// `~/projects/foo`, which would also fail a naive `"agents"`
/// segment-scan that happened to match `~/projects/agents/foo`).
/// Caller logs + skips — agent stays in SQLite, just not in the
/// cross-version dropdown.
fn agent_instance_to_record(
    inst: &AgentInstance,
    agents_root: &Path,
) -> Result<crate::registry::NamedAgentRecord, String> {
    use crate::registry::{NamedAgentRecord, NamedAgentRecordV1};
    let rel = relative_workdir(&inst.working_directory, agents_root).ok_or_else(|| {
        format!(
            "working_directory {:?} is not under {:?}",
            inst.working_directory,
            agents_root.display()
        )
    })?;
    let version = env!("CARGO_PKG_VERSION").to_string();
    let data = NamedAgentRecordV1 {
        instance_id: inst.id.clone(),
        instance_name: inst.instance_name.clone(),
        definition_id: inst.definition_id.clone(),
        identity_id: empty_to_none(&inst.identity_id),
        memory_id: empty_to_none(&inst.memory_id),
        // v2: carry the provider session id so a global/cross-channel
        // record can `--resume` without joining the current channel's SQLite.
        session_id: empty_to_none(&inst.session_id),
        working_dir: rel,
        created_at_ms: inst.created_at,
        last_launched_at_ms: inst.started_at,
        created_by_version: version.clone(),
        last_launched_by_version: version,
    };
    // Lazy schema bump: v2 only when session_id is set, so session-less
    // records stay readable by v1 binaries (see schema::min_schema_version).
    Ok(NamedAgentRecord {
        schema_version: data.min_schema_version(),
        data,
    })
}

fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Express `abs` as a path relative to `agents_root`. Returns `None`
/// when `abs` is empty, not under `agents_root`, or after stripping
/// resolves to an empty path. Anchors against the **resolved** shared
/// root (passed in by the caller) — never scans for a path segment
/// named "agents", which would match unrelated user directories like
/// `/home/me/projects/agents/foo`.
fn relative_workdir(abs: &str, agents_root: &Path) -> Option<String> {
    if abs.is_empty() {
        return None;
    }
    let p = std::path::Path::new(abs);
    let rel = p.strip_prefix(agents_root).ok()?;
    // Reject empties + traversals (defense in depth — strip_prefix
    // already rules out `..` escapes, but the registry's own validator
    // re-checks).
    let s = rel.to_string_lossy().to_string();
    if s.is_empty() || s == "." {
        return None;
    }
    Some(s)
}

impl Store {
    /// Mirror a `db_agent_instances` mutation into the cross-version
    /// registry. Only fires for **named** rows. Routes by
    /// `display_hidden` so the registry file ends up in the tree
    /// matching SQLite's dropdown filter:
    ///
    /// - hidden = true  → upsert (atomic write to active/) then
    ///   retire (atomic rename to retired/). Net: file lives in
    ///   `retired/<id>.json` with the freshest content. Prevents
    ///   `instance_update` on a previously-hidden row from
    ///   resurrecting an active registry file, AND keeps the
    ///   retired tombstone's content current.
    /// - hidden = false → unretire (no-op if not retired) then
    ///   upsert. Net: file in `active/<id>.json`, no orphan retired.
    ///
    /// Failures are logged, never propagated: SQLite remains
    /// authoritative.
    pub(super) fn registry_upsert_if_named(&self, inst: &AgentInstance) {
        // Mirror filter: only registers named rows. Pre-Option-E this
        // also excluded continuation rows (parent_instance_id != '')
        // so the registry-sourced read path wouldn't surface chained
        // resumes as duplicate dropdown rows. Under Option E, the
        // session zone is anchored on definition_id, so a continuation
        // row IS the most-recent named instance — exactly what we want
        // visible. `instance_list_named` (the SQLite-sourced read
        // path) dropped its parent_instance_id filter in the
        // 2026-05-24 picker-visibility fix; the registry mirror keeps
        // its filter here for now since the registry-sourced read path
        // doesn't have the dedup-by-(definition_id, instance_name)
        // affordance the SQLite ORDER BY/LIMIT provides. Follow-up
        // PR will land cross-version dedup so this filter can drop too.
        if inst.instance_name.is_empty() || !inst.parent_instance_id.is_empty() {
            return;
        }
        let Some(reg) = self.shared_agent_registry() else {
            return;
        };
        // Anchor the relative working_directory on the CURRENT channel's
        // agents dir, not the registry's own parent — once P0.3 re-roots the
        // registry under ~/.agentmux/shared/, its parent no longer coincides
        // with channels/<ch>/agents/. `registry_agents_base()` returns the
        // explicit channel dir (AGENTMUX_AGENTS_DIR) and falls back to the
        // registry parent for tests / pre-re-root layouts.
        let Some(agents_root) = self.registry_agents_base() else {
            tracing::warn!("registry: no agents base — skipping mirror");
            return;
        };
        let rec = match agent_instance_to_record(inst, &agents_root) {
            Ok(rec) => rec,
            Err(e) => {
                tracing::warn!(
                    instance_id = %inst.id,
                    error = %e,
                    "registry: instance not representable as record, skipping mirror"
                );
                return;
            }
        };

        // If the row was previously hidden, the file lives in
        // `retired/`. Move it back to active before upserting so
        // upsert's merge-preserves-unknown-fields path operates on
        // the right file (and we never leave dangling retired files).
        if let Err(e) = reg.unretire(&inst.id) {
            tracing::warn!(
                instance_id = %inst.id,
                error = %e,
                "registry: failed to unretire row before upsert"
            );
        }

        if let Err(e) = reg.upsert(&rec) {
            tracing::warn!(
                instance_id = %inst.id,
                error = %e,
                "registry: failed to mirror instance_create/update"
            );
            return;
        }

        // After the upsert, move into retired/ if the row is hidden.
        // Combined: hidden row's tombstone always has up-to-date
        // content, and active/ never carries a hidden row.
        if inst.display_hidden {
            if let Err(e) = reg.retire(&inst.id) {
                tracing::warn!(
                    instance_id = %inst.id,
                    error = %e,
                    "registry: failed to retire hidden row post-upsert"
                );
            }
        }
    }
}
