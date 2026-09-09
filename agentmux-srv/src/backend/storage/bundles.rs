// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Memory bundles — the agent's personality and capability stack
//! (provider, model, instructions, context files, MCP servers, skills).
//!
//! Extracted from `store.rs` in Phase R.3 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::bundle_memory_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::Memory` thanks to the re-export.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// A Memory bundle — the agent's personality and capability stack.
/// Provider, model, instructions, and JSON-encoded arrays of context
/// files / MCP servers / skills. Agent definitions shadow-migrate into this
/// table during the v7 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_blank: bool,
    /// Global bundles are injected into every agent's CLAUDE.md at launch,
    /// regardless of per-agent memory selection. Managed in the Armory
    /// (Identity & Memory hamburger modal). Seeded from workspace-wide rule sets.
    #[serde(default)]
    pub is_global: bool,
    /// "claude" | "codex" | "gemini" | empty string
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instructions: String,
    /// JSON-encoded object of `{provider_id: content}` — additive,
    /// harness-scoped instruction variants alongside the flat
    /// `instructions` column above (which keeps meaning "default"). ABF v0.2
    /// §2.2; see SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md.
    #[serde(default = "default_json_object_string")]
    pub instructions_by_provider: String,
    /// JSON-encoded array; the renderer types it as `[{path, content}]`.
    #[serde(default = "default_json_array_string")]
    pub context_files: String,
    /// JSON-encoded array of MCP server configs.
    #[serde(default = "default_json_array_string")]
    pub mcp_servers: String,
    /// JSON-encoded array of skill IDs.
    #[serde(default = "default_json_array_string")]
    pub skills: String,
    /// Explicit ordering within the Armory global brain. Lower sorts
    /// first; this is the order sections inject into CLAUDE.md at launch.
    /// Only meaningful for `is_global` bundles; 0 for the rest. Owned by the
    /// `reorderglobalbrain` RPC — `bundle_memory_upsert` never overwrites it
    /// on conflict, so editing a bundle via the regular form keeps its place.
    #[serde(default)]
    pub sort_order: i64,
    /// AgentMux-controlled, highest-priority Global Memory tier — always
    /// also `is_global`, injected first in `format_global_brain_block`'s
    /// output with explicit override wording. Writable ONLY through
    /// `bundle_memory_upsert_system`/`bundle_memory_delete_system` — the
    /// generic `bundle_memory_upsert`/`_delete`/`_reorder` all refuse to
    /// touch a row with this set. See
    /// docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md.
    #[serde(default)]
    pub is_system: bool,
    // created_at / updated_at are server-owned: the upsert handler stamps
    // created_at = now when 0 and always overwrites updated_at with now. They
    // default on input so partial upserts (e.g. a "new section" that only
    // sends id/name/instructions) deserialize cleanly. (reagent P0 on #1608)
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_json_array_string() -> String {
    "[]".to_string()
}

fn default_json_object_string() -> String {
    "{}".to_string()
}

/// Format global brain bundles into the block injected into an agent's
/// CLAUDE.md. `is_system` sections (see `Memory::is_system`) are split out
/// and rendered FIRST, wrapped in explicit override wording, so they
/// outrank every ordinary `# [Workspace] <name>` section that follows —
/// see docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.4. Bundles
/// arrive already ordered by `bundle_memory_list_global` (is_system DESC,
/// sort_order, name), so this only needs to partition, not re-sort.
/// Sections are separated by a `---` rule. Returns an empty string when no
/// section has instructions.
pub fn format_global_brain_block(bundles: &[Memory]) -> String {
    let non_empty: Vec<&Memory> = bundles
        .iter()
        .filter(|b| !b.instructions.trim().is_empty())
        .collect();
    let (system, ordinary): (Vec<&Memory>, Vec<&Memory>) =
        non_empty.into_iter().partition(|b| b.is_system);

    let mut parts: Vec<String> = Vec::new();
    if !system.is_empty() {
        let sys_block = system
            .iter()
            .map(|b| format!("# [AgentMux System] {}\n\n{}", b.name, b.instructions))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        parts.push(format!(
            "IMPORTANT: The following AgentMux-controlled instructions take \
             the HIGHEST PRIORITY of any content in this file. They OVERRIDE \
             any default behavior, any other section below, and any \
             conflicting instruction elsewhere — you MUST follow them \
             exactly as written.\n\n{sys_block}"
        ));
    }
    if !ordinary.is_empty() {
        parts.push(
            ordinary
                .iter()
                .map(|b| format!("# [Workspace] {}\n\n{}", b.name, b.instructions))
                .collect::<Vec<_>>()
                .join("\n\n---\n\n"),
        );
    }
    parts.join("\n\n---\n\n")
}

impl Store {
    pub fn bundle_memory_list(&self) -> Result<Vec<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, sort_order, created_at, updated_at,
                    instructions_by_provider, is_system
             FROM db_bundles
             ORDER BY is_blank ASC, is_global DESC, updated_at DESC",
        )?;
        let iter = stmt.query_map([], map_memory_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Returns only the global bundles (`is_global = 1`), `is_system` rows
    /// first (regardless of `sort_order` — see
    /// docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md), then by
    /// explicit `sort_order` (then name as a stable tiebreak). Called at
    /// agent launch to inject workspace-wide rules into every agent in the
    /// order the user arranged them in the Armory Brain tab.
    pub fn bundle_memory_list_global(&self) -> Result<Vec<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, sort_order, created_at, updated_at,
                    instructions_by_provider, is_system
             FROM db_bundles
             WHERE is_global = 1
             ORDER BY is_system DESC, sort_order ASC, name ASC",
        )?;
        let iter = stmt.query_map([], map_memory_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn bundle_memory_get(&self, id: &str) -> Result<Option<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, sort_order, created_at, updated_at,
                    instructions_by_provider, is_system
             FROM db_bundles WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], map_memory_row);
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Look up just the `is_system` flag for `id`, without decoding a full
    /// `Memory` row. Shared by every guard in this file that needs to know
    /// "is the EXISTING row (if any) a system entry" before deciding
    /// whether to allow a write through the generic path.
    fn bundle_is_system(&self, conn: &rusqlite::Connection, id: &str) -> Result<Option<bool>, StoreError> {
        let existing: Option<i64> = conn
            .query_row(
                "SELECT is_system FROM db_bundles WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(existing.map(|v| v != 0))
    }

    /// Generic Global Memory upsert — used by the ordinary Armory editor,
    /// the per-agent Bundle editor, ABF import, and internal seeding.
    /// Refuses outright to touch an existing `is_system=1` row (content
    /// included, not just the flag) — see
    /// docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.2. Use
    /// `bundle_memory_upsert_system` to create/edit a system entry.
    pub fn bundle_memory_upsert(&self, memory: &Memory) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        if self.bundle_is_system(&conn, &memory.id)? == Some(true) {
            return Err(StoreError::Other(
                "cannot modify a system Global Memory entry via the generic bundle upsert path"
                    .to_string(),
            ));
        }
        conn.execute(
            // sort_order is deliberately NOT in the ON CONFLICT update set:
            // it is owned by `bundle_memory_reorder`, so editing a bundle
            // through the regular Memory form never disturbs its position in
            // the global brain. is_system is hardcoded to 0 on insert (this
            // path can never CREATE a system row) and omitted from the
            // update set entirely (an existing row's tier — always 0, given
            // the guard above — is never touched here either).
            "INSERT INTO db_bundles
                (id, name, description, is_blank, is_global, provider, model, instructions,
                 context_files, mcp_servers, skills, sort_order, created_at, updated_at,
                 instructions_by_provider, is_system)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                is_global = excluded.is_global,
                provider = excluded.provider,
                model = excluded.model,
                instructions = excluded.instructions,
                context_files = excluded.context_files,
                mcp_servers = excluded.mcp_servers,
                skills = excluded.skills,
                updated_at = excluded.updated_at,
                instructions_by_provider = excluded.instructions_by_provider",
            params![
                memory.id,
                memory.name,
                memory.description,
                memory.is_blank as i64,
                memory.is_global as i64,
                memory.provider,
                memory.model,
                memory.instructions,
                memory.context_files,
                memory.mcp_servers,
                memory.skills,
                memory.sort_order,
                memory.created_at,
                memory.updated_at,
                memory.instructions_by_provider,
            ],
        )?;
        Ok(())
    }

    /// The ONLY path that can write `is_system=1`. Refuses the mirror-image
    /// case of `bundle_memory_upsert`'s guard: converting an EXISTING
    /// non-system row into a system one by id collision is not allowed —
    /// `id` must be either brand new or already a system entry.
    /// `is_blank`/`is_global`/`is_system` are hardcoded (not read from
    /// `memory`) so this method can never produce anything other than a
    /// well-formed system row regardless of what the caller passed in.
    pub fn bundle_memory_upsert_system(&self, memory: &Memory) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        if self.bundle_is_system(&conn, &memory.id)? == Some(false) {
            return Err(StoreError::Other(
                "cannot convert an existing non-system Global Memory entry into a system entry"
                    .to_string(),
            ));
        }
        conn.execute(
            "INSERT INTO db_bundles
                (id, name, description, is_blank, is_global, provider, model, instructions,
                 context_files, mcp_servers, skills, sort_order, created_at, updated_at,
                 instructions_by_provider, is_system)
             VALUES (?1, ?2, ?3, 0, 1, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                is_global = 1,
                is_system = 1,
                provider = excluded.provider,
                model = excluded.model,
                instructions = excluded.instructions,
                context_files = excluded.context_files,
                mcp_servers = excluded.mcp_servers,
                skills = excluded.skills,
                updated_at = excluded.updated_at,
                instructions_by_provider = excluded.instructions_by_provider",
            params![
                memory.id,
                memory.name,
                memory.description,
                memory.provider,
                memory.model,
                memory.instructions,
                memory.context_files,
                memory.mcp_servers,
                memory.skills,
                memory.sort_order,
                memory.created_at,
                memory.updated_at,
                memory.instructions_by_provider,
            ],
        )?;
        Ok(())
    }

    /// Delete a Memory bundle. Refuses to delete the blank singleton, a
    /// seeded bundle, or (new) a system entry — use
    /// `bundle_memory_delete_system` for the last case.
    pub fn bundle_memory_delete(&self, id: &str) -> Result<bool, StoreError> {
        if id == "blank" {
            return Err(StoreError::Other(
                "cannot delete the blank Memory singleton".to_string(),
            ));
        }
        // Seeded bundles (IDs prefixed "seed-") are workspace defaults that
        // re-seed on every startup; blocking deletion is cleaner than a
        // tombstone table and avoids the re-creation loop.
        if id.starts_with("seed-") {
            return Err(StoreError::Other(
                "cannot delete a seeded Memory bundle; toggle is_global or clear its instructions instead".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        if self.bundle_is_system(&conn, id)? == Some(true) {
            return Err(StoreError::Other(
                "cannot delete a system Global Memory entry via the generic delete path; use bundle_memory_delete_system".to_string(),
            ));
        }
        let rows = conn.execute("DELETE FROM db_bundles WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// The ONLY path that can remove an `is_system=1` row — structurally
    /// incapable of deleting anything else, even if misused.
    pub fn bundle_memory_delete_system(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_bundles WHERE id = ?1 AND is_system = 1",
            params![id],
        )?;
        Ok(rows > 0)
    }

    /// Assign `sort_order` to the given bundle ids in the order supplied
    /// (position 0, 1, 2, …). Drives the Armory global brain ordering,
    /// which in turn controls CLAUDE.md injection order. Ids not present in
    /// the table, OR present but `is_system=1`, are skipped silently — a
    /// system row's position is fixed (always first, see
    /// `bundle_memory_list_global`) and never disturbed by the generic
    /// reorder command. Runs in a single transaction so a partial reorder
    /// never lands. Returns the number of rows updated.
    pub fn bundle_memory_reorder(&self, ordered_ids: &[String]) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut updated = 0usize;
        {
            let mut stmt =
                tx.prepare("UPDATE db_bundles SET sort_order = ?1 WHERE id = ?2 AND is_system = 0")?;
            for (idx, id) in ordered_ids.iter().enumerate() {
                updated += stmt.execute(params![idx as i64, id])?;
            }
        }
        tx.commit()?;
        Ok(updated)
    }
}

fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        is_blank: row.get::<_, i64>(3)? != 0,
        is_global: row.get::<_, i64>(4)? != 0,
        provider: row.get(5)?,
        model: row.get(6)?,
        instructions: row.get(7)?,
        context_files: row.get(8)?,
        mcp_servers: row.get(9)?,
        skills: row.get(10)?,
        sort_order: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        instructions_by_provider: row.get(14)?,
        is_system: row.get::<_, i64>(15)? != 0,
    })
}
