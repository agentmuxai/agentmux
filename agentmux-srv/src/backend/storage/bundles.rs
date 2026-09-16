// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bundles — the agent's personality and capability stack
//! (provider, model, instructions, context files, MCP servers, skills).
//!
//! Extracted from `store.rs` in Phase R.3 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::bundle_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::Bundle` thanks to the re-export.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::bundle_versions::{bundle_version_insert_tx, BundleVersion};
use super::error::StoreError;
use super::store::Store;

/// A Bundle — the agent's personality and capability stack.
/// Provider, model, instructions, and JSON-encoded arrays of context
/// files / MCP servers / skills. Agent definitions shadow-migrate into this
/// table during the v7 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_blank: bool,
    /// Global bundles are injected into every agent's CLAUDE.md at launch,
    /// regardless of per-agent memory selection. Managed in the Armory
    /// (Identity & Bundle hamburger modal). Seeded from workspace-wide rule sets.
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
    /// Explicit ordering within the Armory global bundles. Lower sorts
    /// first; this is the order sections inject into CLAUDE.md at launch.
    /// Only meaningful for `is_global` bundles; 0 for the rest. Owned by the
    /// `reorderglobalbrain` RPC — `bundle_upsert` never overwrites it
    /// on conflict, so editing a bundle via the regular form keeps its place.
    #[serde(default)]
    pub sort_order: i64,
    /// AgentMux-controlled, highest-priority Global Bundle tier — always
    /// also `is_global`, injected first in `format_global_bundle_block`'s
    /// output with explicit override wording. Writable ONLY through
    /// `bundle_upsert_system`/`bundle_delete_system` — the
    /// generic `bundle_upsert`/`_delete`/`_reorder` all refuse to
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

/// Outcome of `Store::bundle_reseed_system_if_owned` — see that method's own
/// doc comment for what each variant means and why the decision is made
/// atomically rather than by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleReseedOutcome {
    Created,
    Updated,
    Unchanged,
    SkippedLocalEdit,
    /// This write's manifest generation (see `manifest_generation_from_
    /// source_detail`) is older than the generation already recorded for
    /// this row — an older AgentMux process must never downgrade a newer
    /// seed. See `bundle_reseed_system_if_owned`'s own doc comment.
    SkippedOlderGeneration,
    /// The row doesn't exist, but a retirement tombstone (`RETIREMENT_
    /// TOMBSTONE_SOURCE`) recorded by a newer manifest generation says it
    /// was deliberately pruned — this caller's own (older) manifest must
    /// not resurrect it. See `bundle_reseed_system_if_owned`'s own doc
    /// comment.
    SkippedRetired,
}

/// The `source` sentinel `bundle_delete_system_if_owned` stamps on the
/// tombstone version row it appends after deleting a row — distinguishes a
/// deliberate, generation-aware retirement (pruned because a manifest no
/// longer lists this id) from an ordinary human delete via the Armory UI
/// (`bundle_delete_system`, which records no version at all). `bundle_
/// reseed_system_if_owned`'s missing-row branch checks for this sentinel
/// specifically — see that method's own doc comment.
const RETIREMENT_TOMBSTONE_SOURCE: &str = "operator_config_seed_retired";

/// Extracts a `{"manifest_version": N}` field from a version row's
/// `source_detail` JSON, defaulting to `0` when absent or unparseable — so a
/// pre-existing row seeded before this field existed is always treated as
/// generation 0, never blocking a real (>=1) manifest generation from
/// writing over it. Shared convention between `bundle_reseed_system_if_
/// owned`'s own generation guard and any seeder populating `source_detail`
/// (see `operator_config_seed.rs`).
fn manifest_generation_from_source_detail(source_detail: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(source_detail)
        .ok()
        .and_then(|v| v.get("manifest_version").and_then(|g| g.as_u64()))
        .unwrap_or(0)
}

fn default_json_object_string() -> String {
    "{}".to_string()
}

/// Format global bundles into the block injected into an agent's
/// CLAUDE.md. `is_system` sections (see `Bundle::is_system`) are split out
/// and rendered FIRST, wrapped in explicit override wording, so they
/// outrank every ordinary `# [Workspace] <name>` section that follows —
/// see docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.4. Bundles
/// arrive already ordered by `bundle_list_global` (is_system DESC,
/// sort_order, name), so this only needs to partition, not re-sort.
/// Sections are separated by a `---` rule. Returns an empty string when no
/// section has instructions.
pub fn format_global_bundle_block(bundles: &[Bundle]) -> String {
    let non_empty: Vec<&Bundle> = bundles
        .iter()
        .filter(|b| !b.instructions.trim().is_empty())
        .collect();
    let (system, ordinary): (Vec<&Bundle>, Vec<&Bundle>) =
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
    pub fn bundle_list(&self) -> Result<Vec<Bundle>, StoreError> {
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
    /// order the user arranged them in the Armory Global section.
    pub fn bundle_list_global(&self) -> Result<Vec<Bundle>, StoreError> {
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

    pub fn bundle_get(&self, id: &str) -> Result<Option<Bundle>, StoreError> {
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
    /// `Bundle` row. Shared by every guard in this file that needs to know
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

    /// Generic Global Bundle upsert — used by the ordinary Armory editor,
    /// the per-agent Bundle editor, ABF import, and internal seeding.
    /// Refuses outright to touch an existing `is_system=1` row (content
    /// included, not just the flag) — see
    /// docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.2. Use
    /// `bundle_upsert_system` to create/edit a system entry.
    pub fn bundle_upsert(&self, memory: &Bundle) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        if self.bundle_is_system(&conn, &memory.id)? == Some(true) {
            return Err(StoreError::Other(
                "cannot modify a system Global Memory entry via the generic bundle upsert path"
                    .to_string(),
            ));
        }
        conn.execute(
            // sort_order is deliberately NOT in the ON CONFLICT update set:
            // it is owned by `bundle_reorder`, so editing a bundle
            // through the regular Bundle form never disturbs its position in
            // the global bundles. is_system is hardcoded to 0 on insert (this
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

    /// Same as `bundle_upsert`, but when `memory.is_global` is true, also
    /// appends a `db_bundle_versions` row for it — atomically, in the SAME
    /// transaction as the `db_bundles` write, not as two separate calls.
    /// codex P2, PR #3237: two separate lock/transaction acquisitions (the
    /// original shape — call `bundle_upsert`, then separately call
    /// `bundle_version_insert`) let a concurrent writer interleave between
    /// them: writer A upserts, writer B upserts AND versions, then A's own
    /// (now-stale) version-insert lands last — the history would then claim
    /// A's content is current while the live row actually holds B's. One
    /// transaction makes that interleaving impossible.
    ///
    /// `written_by` is the caller's own TRUSTED identity (an agent's real
    /// `AGENTMUX_AGENT_ID`, or `"armory-ui"` for the human-facing Armory
    /// editor) — see `BundleVersion::written_by`'s own doc comment for why
    /// this is a separate field from `source`/`source_detail`. Returns
    /// `None` (no version recorded) when `memory.is_global` is false —
    /// versioning is scoped to Global Memory specifically, matching this
    /// table's whole purpose; a private, per-agent bundle upsert still
    /// writes normally but has nothing to audit here.
    ///
    /// Shares `bundle_upsert`'s exact guard (refuses an existing
    /// `is_system=1` row) and exact `db_bundles` SQL — kept as a literal
    /// duplicate rather than having one call the other, since `self.conn`
    /// is a plain (non-reentrant) `Mutex`: calling `bundle_upsert` (which
    /// takes its own lock) from inside a transaction already holding that
    /// same lock would deadlock.
    pub fn bundle_upsert_with_version(
        &self,
        memory: &Bundle,
        written_by: &str,
        source: &str,
        source_detail: &str,
    ) -> Result<Option<BundleVersion>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let existing_is_system: Option<i64> = tx
            .query_row(
                "SELECT is_system FROM db_bundles WHERE id = ?1",
                params![memory.id],
                |r| r.get(0),
            )
            .optional()?;
        if existing_is_system == Some(1) {
            return Err(StoreError::Other(
                "cannot modify a system Global Memory entry via the generic bundle upsert path"
                    .to_string(),
            ));
        }

        tx.execute(
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

        let version = if memory.is_global {
            Some(bundle_version_insert_tx(
                &tx,
                &memory.id,
                &memory.name,
                &memory.instructions,
                source,
                source_detail,
                written_by,
            )?)
        } else {
            None
        };

        tx.commit()?;
        Ok(version)
    }

    /// The ONLY path that can write `is_system=1`. Refuses the mirror-image
    /// case of `bundle_upsert`'s guard: converting an EXISTING
    /// non-system row into a system one by id collision is not allowed —
    /// `id` must be either brand new or already a system entry.
    /// `is_blank`/`is_global`/`is_system` are hardcoded (not read from
    /// `memory`) so this method can never produce anything other than a
    /// well-formed system row regardless of what the caller passed in.
    pub fn bundle_upsert_system(&self, memory: &Bundle) -> Result<(), StoreError> {
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

    /// Same as `bundle_upsert_system`, but also appends a `db_bundle_versions`
    /// row in the SAME transaction — the system-tier counterpart of
    /// `bundle_upsert_with_version`, for the same atomicity reason (codex P2,
    /// PR #3237). Always records a version (unlike the ordinary method, this
    /// one never returns `None`): a system row is unconditionally
    /// `is_global=1`, so there is never a case where versioning would not
    /// apply. See docs/specs/SPEC_SYSTEM_TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md
    /// — this is what lets `operator_config_seed`'s startup reseed tell "did
    /// AgentMux's own seeder write this last, or did a human edit it via the
    /// Armory UI" apart, via the returned/stored `written_by`.
    ///
    /// Shares `bundle_upsert_system`'s exact guard and SQL, duplicated for
    /// the same non-reentrant-`Mutex` reason `bundle_upsert_with_version`
    /// documents on itself.
    pub fn bundle_upsert_system_with_version(
        &self,
        memory: &Bundle,
        written_by: &str,
        source: &str,
        source_detail: &str,
    ) -> Result<BundleVersion, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let existing_is_system: Option<i64> = tx
            .query_row(
                "SELECT is_system FROM db_bundles WHERE id = ?1",
                params![memory.id],
                |r| r.get(0),
            )
            .optional()?;
        if existing_is_system == Some(0) {
            return Err(StoreError::Other(
                "cannot convert an existing non-system Global Memory entry into a system entry"
                    .to_string(),
            ));
        }

        tx.execute(
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

        let version = bundle_version_insert_tx(
            &tx,
            &memory.id,
            &memory.name,
            &memory.instructions,
            source,
            source_detail,
            written_by,
        )?;

        tx.commit()?;
        Ok(version)
    }

    /// Same as `bundle_upsert_system_with_version`, but skips recording a
    /// version when the write is byte-identical (`name` + `instructions`) to
    /// what's already stored — the case where an operator opens a seeded
    /// Operator Config entry in the Armory UI and clicks Save without
    /// changing anything (the UI does not suppress this request). Recording
    /// a version for a true no-op would permanently mark the row as
    /// human-owned even though nothing actually changed, breaking `bundle_
    /// reseed_system_if_owned`'s ownership signal for no reason (Codex P2,
    /// PR #3244).
    ///
    /// The existing-content comparison happens in the SAME transaction as
    /// the resulting write, for the identical check-then-act reason `bundle_
    /// reseed_system_if_owned` documents on itself — an earlier revision of
    /// `upsertsystemmemory` (`bundle.rs`) composed this from a separate
    /// `bundle_get` call plus a conditional branch, which raced a concurrent
    /// writer between the read and the write (ReAgent P2, PR #3244).
    ///
    /// Returns `Ok(None)` when the save was a no-op (still refreshes every
    /// other field via the plain upsert; just records no version); `Ok(Some(
    /// version))` otherwise, including for a brand-new row (never a no-op).
    pub fn bundle_upsert_system_if_changed(
        &self,
        memory: &Bundle,
        written_by: &str,
        source: &str,
        source_detail: &str,
    ) -> Result<Option<BundleVersion>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let existing: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT name, instructions, is_system FROM db_bundles WHERE id = ?1",
                params![memory.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        if let Some((_, _, is_system)) = &existing {
            if *is_system == 0 {
                return Err(StoreError::Other(
                    "cannot convert an existing non-system Global Memory entry into a system entry"
                        .to_string(),
                ));
            }
        }

        let is_noop = existing
            .as_ref()
            .is_some_and(|(name, instructions, _)| *name == memory.name && *instructions == memory.instructions);

        tx.execute(
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

        let version = if is_noop {
            None
        } else {
            Some(bundle_version_insert_tx(
                &tx,
                &memory.id,
                &memory.name,
                &memory.instructions,
                source,
                source_detail,
                written_by,
            )?)
        };

        tx.commit()?;
        Ok(version)
    }

    /// Atomically decide-and-apply an Operator Config reseed for one system
    /// row — the ownership check (is this row still owned by AgentMux's own
    /// seeder, or has a human/another writer claimed it) and the resulting
    /// write happen inside the SAME transaction, not as two separate `Store`
    /// calls composed by the caller. A caller-composed version (read latest
    /// version via `bundle_version_list`, decide, then separately call
    /// `bundle_upsert_system_with_version`) is a check-then-act race: a
    /// concurrent writer — a human editing this row via the Armory UI, or
    /// another AgentMux instance's own startup reseed, since multiple
    /// instances can run in parallel by design — could land a new version
    /// between the read and the write, and the caller's decision would be
    /// based on data that was already stale by the time it acted (ReAgent
    /// P1, PR #3244).
    ///
    /// `seeder_identity` is the reserved `written_by` value that marks a row
    /// as still owned by the seeder (see `operator_config_seed::WRITTEN_BY`)
    /// — a latest version written by anything else means a human (or some
    /// other writer) has claimed this row, and it is left alone.
    ///
    /// - No existing `db_bundles` row at all -> (re)create, UNLESS the
    ///   latest version for this id is a `RETIREMENT_TOMBSTONE_SOURCE`
    ///   marker whose recorded generation is newer than `source_detail`'s
    ///   own (`SkippedRetired` in that case) — see `bundle_delete_system_
    ///   if_owned`'s own doc comment for why the tombstone exists: without
    ///   it, an older AgentMux build (or a rollback) sharing this store.db
    ///   would read "no row" as "never existed" and resurrect content a
    ///   newer build's prune deliberately retired (ReAgent/Codex P2, PR
    ///   #3244). An ordinary human delete (`bundle_delete_system`, which
    ///   records no version at all) leaves no tombstone, so a manually
    ///   deleted entry still comes back exactly as before — only a
    ///   generation-aware prune leaves a tombstone behind.
    /// - Existing row, latest version's `written_by != seeder_identity` ->
    ///   `SkippedLocalEdit`, no write at all.
    /// - Existing row, latest version's `content_hash` already matches the
    ///   target (`name`+`instructions`) -> `Unchanged`, no write.
    /// - Otherwise -> upsert + version insert, `Updated`.
    pub fn bundle_reseed_system_if_owned(
        &self,
        memory: &Bundle,
        seeder_identity: &str,
        source: &str,
        source_detail: &str,
    ) -> Result<BundleReseedOutcome, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let existing: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT is_system, name, instructions FROM db_bundles WHERE id = ?1",
                params![memory.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let row_exists = existing.is_some();

        if let Some((0, _, _)) = existing {
            return Err(StoreError::Other(
                "cannot convert an existing non-system Global Memory entry into a system entry"
                    .to_string(),
            ));
        }

        if let Some((_, live_name, live_instructions)) = &existing {
            // Ordered by rowid (insertion sequence), NOT created_at: unlike
            // bundle_version_list's own history ordering (a display
            // concern, where wall-clock order is what a human expects to
            // see), this ownership decision has a real behavioral
            // consequence if it picks the wrong "latest" row. A backward
            // system-clock jump (e.g. correcting a VM whose clock was
            // ahead) between two writes to the same bundle_id would give
            // the chronologically-later write a SMALLER created_at, so
            // `ORDER BY created_at DESC` could keep selecting the older
            // row as "latest" — silently defeating the ownership
            // guarantee this whole method exists for. rowid is monotonic
            // per insert regardless of wall-clock time. Codex P2, PR #3244.
            let latest: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT written_by, content_hash, source_detail FROM db_bundle_versions
                     WHERE bundle_id = ?1
                     ORDER BY rowid DESC
                     LIMIT 1",
                    params![memory.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((written_by, content_hash, stored_source_detail)) = latest {
                // The live db_bundles row must match what the latest
                // version CLAIMS is there before `written_by` can be
                // trusted at all — a compatible-schema OLDER build sharing
                // this store might still run a pre-PR-#3244
                // `upsertsystemmemory` handler that calls the unversioned
                // `bundle_upsert_system` directly, changing the live row
                // without ever touching `db_bundle_versions`. In that case
                // the latest version would still (wrongly) show the seeder
                // as the last writer, even though a human's edit is
                // sitting live and unrecorded. Codex P2, PR #3244.
                let live_hash = super::bundle_versions::content_hash(live_name, live_instructions);
                if live_hash != content_hash {
                    return Ok(BundleReseedOutcome::SkippedLocalEdit);
                }
                if written_by != seeder_identity {
                    return Ok(BundleReseedOutcome::SkippedLocalEdit);
                }
                let target_hash = super::bundle_versions::content_hash(&memory.name, &memory.instructions);
                let stored_generation = manifest_generation_from_source_detail(&stored_source_detail);
                let target_generation = manifest_generation_from_source_detail(source_detail);
                if content_hash == target_hash {
                    // Content is identical, but if THIS call's own manifest
                    // generation is strictly newer than what's recorded,
                    // that fact must still be persisted — otherwise a
                    // concurrently-running build on a LOWER generation
                    // (still not "older" than the stale recorded one) could
                    // later see its own differing content as valid to write
                    // and downgrade content a newer generation already
                    // confirmed as current. E.g. generation 3's text
                    // happens to match generation 1's (a revert); without
                    // this, a generation-2 build's own different text would
                    // still pass the generation check against the
                    // never-advanced "1" and overwrite generation 3's
                    // result. Codex P2, PR #3244.
                    if target_generation > stored_generation {
                        bundle_version_insert_tx(
                            &tx,
                            &memory.id,
                            &memory.name,
                            &memory.instructions,
                            source,
                            source_detail,
                            seeder_identity,
                        )?;
                        // Returning without an explicit commit drops `tx`,
                        // which rolls back — harmless for every OTHER early
                        // return in this method (they make no writes), but
                        // this one just did. Must commit explicitly before
                        // returning.
                        tx.commit()?;
                    }
                    return Ok(BundleReseedOutcome::Unchanged);
                }
                // Content differs, but only from THIS process's point of
                // view — on a machine sharing store.db across multiple
                // AgentMux builds/versions (a supported configuration; see
                // "Multiple Instances Run in Parallel" in this repo's own
                // CLAUDE.md), an older build's own (older) manifest must
                // never overwrite a newer build's already-seeded content
                // just because both processes use the same seeder_identity.
                // Codex P2, PR #3244.
                if target_generation < stored_generation {
                    return Ok(BundleReseedOutcome::SkippedOlderGeneration);
                }
            }
        } else {
            // No db_bundles row — but check for a retirement tombstone
            // before assuming that means "never existed, safe to create."
            // See this method's own doc comment and `bundle_delete_system_
            // if_owned`'s. rowid DESC for the same reason as the row_exists
            // branch above.
            let tombstone: Option<(String, String)> = tx
                .query_row(
                    "SELECT source, source_detail FROM db_bundle_versions
                     WHERE bundle_id = ?1
                     ORDER BY rowid DESC
                     LIMIT 1",
                    params![memory.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((last_source, last_source_detail)) = tombstone {
                if last_source == RETIREMENT_TOMBSTONE_SOURCE {
                    let retired_generation = manifest_generation_from_source_detail(&last_source_detail);
                    let target_generation = manifest_generation_from_source_detail(source_detail);
                    if target_generation < retired_generation {
                        return Ok(BundleReseedOutcome::SkippedRetired);
                    }
                }
            }
        }

        tx.execute(
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

        bundle_version_insert_tx(
            &tx,
            &memory.id,
            &memory.name,
            &memory.instructions,
            source,
            source_detail,
            seeder_identity,
        )?;

        tx.commit()?;
        Ok(if row_exists {
            BundleReseedOutcome::Updated
        } else {
            BundleReseedOutcome::Created
        })
    }

    /// Delete a Bundle. Refuses to delete the blank singleton, a
    /// seeded bundle, or (new) a system entry — use
    /// `bundle_delete_system` for the last case.
    pub fn bundle_delete(&self, id: &str) -> Result<bool, StoreError> {
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
                "cannot delete a system Global Memory entry via the generic delete path; use bundle_delete_system".to_string(),
            ));
        }
        let rows = conn.execute("DELETE FROM db_bundles WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// The ONLY path that can remove an `is_system=1` row — structurally
    /// incapable of deleting anything else, even if misused.
    pub fn bundle_delete_system(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_bundles WHERE id = ?1 AND is_system = 1",
            params![id],
        )?;
        Ok(rows > 0)
    }

    /// All `is_system=1` row ids — the full universe `operator_config_seed`'s
    /// startup prune pass checks its manifest against, to find rows for an
    /// entry AgentMux itself removed or renamed in a later release (Codex
    /// P2, PR #3244). Ids only, not full `Bundle`s: the prune pass needs
    /// nothing else before deciding whether a given id is still in the
    /// manifest.
    pub fn bundle_list_system_ids(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id FROM db_bundles WHERE is_system = 1")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Atomically deletes an `is_system=1` row ONLY if its latest version is
    /// still owned by `seeder_identity` AND that version's own recorded
    /// manifest generation is `<= caller_manifest_version` — the delete-side
    /// counterpart of `bundle_reseed_system_if_owned`'s ownership check,
    /// used to prune a row whose manifest entry AgentMux itself removed in a
    /// later release.
    ///
    /// The generation gate exists for the same reason `bundle_reseed_
    /// system_if_owned` has one: on a machine sharing `store.db` across
    /// multiple AgentMux builds/versions, an OLDER build's own manifest
    /// simply doesn't mention an entry a NEWER build already added — that
    /// absence means "I don't know about this yet," not "this was removed."
    /// Without gating on generation, an older build's prune pass would
    /// delete a newer build's own addition on every one of its startups.
    /// Only a row whose generation this process's OWN manifest has already
    /// caught up to (or exceeds) is eligible for pruning at all.
    ///
    /// A row a human has edited (`written_by` on the latest version is
    /// anything else) is left in place — same "never clobber a local edit"
    /// posture as the reseed path; an orphaned-but-human-owned entry is a
    /// human's to clean up, not this seeder's. A row with NO version
    /// history at all (should not normally happen — every write path that
    /// can create an `is_system` row also records one) is conservatively
    /// treated as NOT owned by the seeder and left alone, rather than
    /// guessing. Returns `true` only when a deletion actually happened.
    ///
    /// Also appends a `RETIREMENT_TOMBSTONE_SOURCE`-marked version row in
    /// the SAME transaction as the delete — without it, an older AgentMux
    /// build (or a rollback to one) sharing this store.db, whose own
    /// manifest still lists this id, would see no `db_bundles` row and
    /// treat that as "never existed, safe to create," silently resurrecting
    /// content this deletion deliberately retired. `bundle_reseed_system_
    /// if_owned`'s missing-row branch checks for this tombstone before
    /// recreating anything (ReAgent/Codex P2, PR #3244).
    ///
    /// Also validates the LIVE row's content against what the latest
    /// version claims before trusting `written_by` at all — the delete-side
    /// mirror of the same check `bundle_reseed_system_if_owned` gained
    /// earlier in this PR. Without it, a compatible-schema OLDER build's
    /// unversioned write (still possible during a rolling upgrade — see
    /// that method's own doc comment) would leave `written_by` stale at
    /// the seeder's identity, and this method would DELETE the unrecorded
    /// edit outright — strictly worse than the overwrite the reseed-path
    /// fix was built to prevent, since deletion cannot be undone from
    /// `db_bundles` alone. ReAgent P1, PR #3244.
    pub fn bundle_delete_system_if_owned(
        &self,
        id: &str,
        seeder_identity: &str,
        caller_manifest_version: u32,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let live: Option<(String, String)> = tx
            .query_row(
                "SELECT name, instructions FROM db_bundles WHERE id = ?1 AND is_system = 1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((live_name, live_instructions)) = live else {
            return Ok(false);
        };

        // rowid, not created_at — see bundle_reseed_system_if_owned's own
        // comment on the same ordering choice.
        let latest: Option<(String, String, String)> = tx
            .query_row(
                "SELECT written_by, content_hash, source_detail FROM db_bundle_versions
                 WHERE bundle_id = ?1
                 ORDER BY rowid DESC
                 LIMIT 1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        let Some((written_by, content_hash, source_detail)) = latest else {
            return Ok(false);
        };
        let live_hash = super::bundle_versions::content_hash(&live_name, &live_instructions);
        if live_hash != content_hash {
            return Ok(false);
        }
        if written_by != seeder_identity {
            return Ok(false);
        }
        let row_generation = manifest_generation_from_source_detail(&source_detail);
        if row_generation > caller_manifest_version as u64 {
            return Ok(false);
        }

        let rows = tx.execute("DELETE FROM db_bundles WHERE id = ?1 AND is_system = 1", params![id])?;
        if rows > 0 {
            let tombstone_detail = format!(r#"{{"manifest_version":{caller_manifest_version}}}"#);
            bundle_version_insert_tx(&tx, id, "", "", RETIREMENT_TOMBSTONE_SOURCE, &tombstone_detail, seeder_identity)?;
        }
        tx.commit()?;
        Ok(rows > 0)
    }

    /// Assign `sort_order` to the given bundle ids in the order supplied
    /// (position 0, 1, 2, …). Drives the Armory global bundles ordering,
    /// which in turn controls CLAUDE.md injection order. Ids not present in
    /// the table, OR present but `is_system=1`, are skipped silently — a
    /// system row's position is fixed (always first, see
    /// `bundle_list_global`) and never disturbed by the generic
    /// reorder command. Runs in a single transaction so a partial reorder
    /// never lands. Returns the number of rows updated.
    pub fn bundle_reorder(&self, ordered_ids: &[String]) -> Result<usize, StoreError> {
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

fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Bundle> {
    Ok(Bundle {
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
