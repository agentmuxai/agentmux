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
    global_agents_root: Option<&Path>,
    channel_agents_base: &Path,
) -> Result<crate::registry::NamedAgentRecord, String> {
    use crate::registry::{NamedAgentRecord, NamedAgentRecordV1};
    // Agent workspaces are GLOBAL (`<home>/agents/<name>`), so anchor on that
    // global root FIRST, falling back to this channel's agents dir only for a
    // legacy in-channel workspace. This MUST match `registry/migrate.rs`'s
    // `row_to_record` — the one-shot migration and this live mirror have to stamp
    // the same `source_agents_base` for a given agent or a reader sees two
    // different bases. Without the global branch, every real (global) workspace
    // failed strip_prefix and the agent was dropped from the registry ("not
    // representable") — the live-write twin of the bug #1393 fixed in the
    // migration.
    let (rel, base): (String, &Path) = global_agents_root
        .and_then(|g| relative_workdir(&inst.working_directory, g).map(|r| (r, g)))
        .or_else(|| {
            relative_workdir(&inst.working_directory, channel_agents_base)
                .map(|r| (r, channel_agents_base))
        })
        .ok_or_else(|| {
            format!(
                "working_directory {:?} is under neither the global agents root {:?} nor the channel base {:?}",
                inst.working_directory,
                global_agents_root.map(|p| p.display().to_string()),
                channel_agents_base.display()
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
        // v3: the agents dir `working_dir` is relative to — the GLOBAL workspace
        // root for a normal agent, or this channel's agents dir for a legacy
        // in-channel workspace. Stored absolute so any channel reconstructs the
        // path via `source_agents_base.join(working_dir)` (P0.4).
        source_agents_base: Some(base.to_string_lossy().to_string()),
        created_at_ms: inst.created_at,
        last_launched_at_ms: inst.started_at,
        created_by_version: version.clone(),
        last_launched_by_version: version,
    };
    // Stamp the lowest envelope schema that faithfully represents the payload
    // (schema::min_schema_version). Since P0.4 always sets source_agents_base,
    // that is v3 for every live-mirrored record — a pre-v3 reader rejects it
    // (intended: better to hide than mis-resolve a cross-channel workdir; the
    // only pre-v3 readers of the GLOBAL registry are pre-P0.4 builds, which
    // ship together with this).
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
        // Agent workspaces are GLOBAL (`<home>/agents/<name>`). Derive `<home>`
        // from the registry root exactly as main.rs does for the one-shot
        // migration (registry = `<home>/shared/agents/registry` → `nth(3)` strips
        // registry→agents→shared = `<home>`), so the live mirror and the migration
        // anchor identically. `None` in shallow test/pre-re-root layouts → the
        // mirror falls back to the per-channel base.
        let global_agents_root = reg.root().ancestors().nth(3).map(|h| h.join("agents"));
        let rec = match agent_instance_to_record(inst, global_agents_root.as_deref(), &agents_root) {
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

    /// Propagate a fresh `session_id` onto the chain-root registry record
    /// when `inst` is a **continuation** row (`parent_instance_id` set).
    ///
    /// `registry_upsert_if_named` above deliberately excludes continuation
    /// rows from the mirror (see its filter comment — kept symmetric with
    /// `instance_list_named`'s legacy dropdown mode so a resume chain
    /// doesn't fragment into N registry files). That's correct for most
    /// fields, but it leaves the registry's `session_id` — the ONE field a
    /// cross-channel/cross-build reader consults to `--resume` this agent —
    /// stuck at whatever the chain head had when first mirrored, never
    /// updated by any later resume. Root cause of
    /// docs/retro/retro-agent-resumed-9-day-stale-session-2026-08-22.md:
    /// reopening the agent in a different channel silently resumed a
    /// stale-but-valid session instead of the live one.
    ///
    /// Read-modify-write against the existing root record (never
    /// constructs a partial one — `Registry::upsert`'s merge overwrites
    /// every field present in the struct). Best-effort: logs and returns
    /// on any lookup/write failure, since the SQLite write already
    /// succeeded and remains authoritative.
    pub(super) fn registry_propagate_continuation_session_id(&self, inst: &AgentInstance) {
        if inst.parent_instance_id.is_empty() {
            return; // chain head — already covered by registry_upsert_if_named.
        }
        let Some(reg) = self.shared_agent_registry() else {
            return;
        };
        let root_id = self.find_chain_root_id(inst);
        if root_id == inst.id {
            return;
        }
        let mut rec = match reg.get(&root_id) {
            Ok(Some(rec)) => rec,
            Ok(None) => return, // root was never named/mirrored — nothing to update.
            Err(e) => {
                tracing::warn!(
                    instance_id = %inst.id,
                    root_id = %root_id,
                    error = %e,
                    "registry: failed to read chain-root record for session_id propagation"
                );
                return;
            }
        };
        let fresh_session_id = empty_to_none(&inst.session_id);
        if rec.data.session_id == fresh_session_id {
            return; // already current — skip the write.
        }
        rec.data.session_id = fresh_session_id;
        rec.schema_version = rec.data.min_schema_version().max(rec.schema_version);
        if let Err(e) = reg.upsert(&rec) {
            tracing::warn!(
                instance_id = %inst.id,
                root_id = %root_id,
                error = %e,
                "registry: failed to propagate continuation session_id to chain root"
            );
        }
    }

    /// Walk `parent_instance_id` upward from `inst` to find the chain
    /// root — the row with no parent, or an orphan whose parent no longer
    /// exists (mirrors `instance_list_named`'s recursive-CTE orphan-as-root
    /// anchor). Bounded to guard against a corrupted cyclic chain; real
    /// chains are a handful of user-driven resumes deep.
    fn find_chain_root_id(&self, inst: &AgentInstance) -> String {
        let mut current = inst.clone();
        for _ in 0..64 {
            if current.parent_instance_id.is_empty() {
                return current.id;
            }
            match self.instance_get(&current.parent_instance_id) {
                Ok(Some(parent)) => current = parent,
                _ => return current.id, // orphan: parent missing/unreadable.
            }
        }
        current.id
    }
}
