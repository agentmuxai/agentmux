// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Two-tier picker — Phase 1 migration (seeded-def → user-agent promote).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::super::helpers::{now_ms, write_zone_file};
use super::super::zone_naming::is_valid_definition_id;

/// Marker file name for the Phase 1 two-tier-picker migration.
///
/// **Vestigial.** Originally gated `migrate_promote_template_sessions_v1`
/// as a one-shot. The 2026-05-24 self-idempotency rework moved gating
/// to the data invariant ("no seeded def has a session zone"), so the
/// migration runs on every startup and is a no-op when the invariant
/// already holds. The constant + `data_dir` parameter on the migration
/// function are kept for API/import compatibility and so the legacy
/// marker file (if present from an earlier portable run) isn't
/// resurrected. Operators may delete the file; the migration ignores
/// it either way.
pub const TEMPLATE_PROMOTE_MARKER_V1: &str = "migration_template_promote_v1.flag";

/// Stats from `migrate_promote_template_sessions_v1`. Logged at INFO.
#[derive(Debug, Clone, Default)]
pub struct TemplatePromoteStats {
    pub templates_scanned: usize,
    pub templates_promoted: usize,
    /// Total archive zones moved across all promotions.
    pub archives_moved: usize,
    /// Total instances repointed via
    /// `wstore.instance_repoint_definition`.
    pub instances_repointed: usize,
    pub failures: usize,
}

/// Phase 1 two-tier picker migration: promote any seeded template that
/// carries a session zone into a fresh user-owned definition, then move
/// its `:current` + `:archive:*` zones onto the new definition_id.
///
/// Why this exists (Q1 = Option C in
/// `docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`):
/// after the picker UI split, clicking a "template" card in the
/// Templates section MUST create a new agent — not silently append to
/// whatever session the user previously ran against that template
/// directly (e.g. "Maks's conversation" living at `agent:claude:current`).
/// Without this migration the template card would either reattach to
/// the existing session (broken — wrong intent) or be effectively
/// non-functional. The migration moves any such pre-existing session
/// out of the template namespace onto a new user-owned definition so
/// the template is pristine post-migration.
///
/// Algorithm:
/// 1. List zone ids; partition the `agent:<id>:current` and
///    `agent:<id>:archive:*` zones by definition id.
/// 2. For each definition id with at least one zone, look up the
///    matching `db_agent_definitions` row.
///    - Skip if missing (zone refers to a deleted definition).
///    - Skip if `is_seeded = 0` (already user-owned — no work).
///    - Otherwise: clone the template into a new user definition
///      (mirrors `agent_def_create_from_template` semantics).
/// 3. Pick the new name: most-recently-active named instance's
///    `instance_name` if any exists, else fall back to the template's
///    own `name`.
/// 4. Move every matching zone (`:current` + every `:archive:*`)
///    from the old defId to the new defId via FileStore's existing
///    write-then-delete pattern.
/// 5. Repoint every `db_agent_instances` row that referenced the old
///    defId to point at the new defId (preserves the
///    `continueOfInstanceId` reattach flow).
///
/// Idempotency: the migration is **self-gated on the data invariant**
/// ("no seeded def has a session zone"). It runs on every startup;
/// when the invariant already holds the inner loop has zero
/// iterations and returns the default stats in sub-ms. There used to
/// be a marker-file gate (`TEMPLATE_PROMOTE_MARKER_V1`), but it
/// produced an "early-marker" failure mode: a portable launched at v
/// N had no seeded-def zones, set the marker, and on v N+1 startups
/// (when seeded-def zones DID exist from prior real use) the marker
/// caused the migration to skip. The 2026-05-24 rework dropped the
/// marker check; this is safe because the seeded-def-with-zone
/// invariant is detectable per-startup at constant cost. `data_dir`
/// is retained for API compatibility.
///
/// Failure mode: per-template errors are logged + counted; we DO NOT
/// abort startup. Errors that prevent a template from being promoted
/// leave its zones in place; the next startup retries (no marker
/// gate to block retry).
pub fn migrate_promote_template_sessions_v1(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    _data_dir: &Path,
) -> TemplatePromoteStats {

    let mut stats = TemplatePromoteStats::default();

    let all_zones = match filestore.get_all_zone_ids() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "template_promote migration: get_all_zone_ids failed; aborting (will retry next start)"
            );
            return stats;
        }
    };

    // Group zone ids by definition id. A zone counts if it matches
    // `agent:<id>:current` OR `agent:<id>:archive:<ts>`. Anything else
    // (e.g. legacy per-block zones the prior migration didn't sweep)
    // is ignored by this migration.
    let mut per_def_zones: HashMap<String, Vec<String>> = HashMap::new();
    for zone in &all_zones {
        let rest = match zone.strip_prefix("agent:") {
            Some(r) => r,
            None => continue,
        };
        // `<defId>:current` or `<defId>:archive:<ts>`
        let (def_id, tail) = match rest.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        if !is_valid_definition_id(def_id) {
            continue;
        }
        let is_current = tail == "current";
        let is_archive = tail.starts_with("archive:");
        if !is_current && !is_archive {
            continue;
        }
        per_def_zones
            .entry(def_id.to_string())
            .or_default()
            .push(zone.clone());
    }

    // Fetch all definitions ONCE so per-template lookups don't re-hit
    // SQLite in a loop.
    let defs = match wstore.agent_def_list() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "template_promote migration: agent_def_list failed; aborting (will retry next start)"
            );
            return stats;
        }
    };

    for (old_def_id, zones) in per_def_zones {
        // Look up the definition row this zone is bound to.
        let template = match defs.iter().find(|d| d.id == old_def_id) {
            Some(d) => d,
            None => {
                // Zone points at a deleted definition — leave it
                // alone; a future GC pass can clean orphans.
                continue;
            }
        };
        // Only seeded templates need promotion. User-owned defs are
        // already on the new model.
        if template.is_seeded != 1 {
            continue;
        }
        stats.templates_scanned += 1;

        // Pick the new agent name: most-recently-active named instance
        // for this template, else fall back to the template's own name.
        // `instance_list_named` already filters to non-hidden + named
        // rows + sorts by `started_at DESC`, so the first row is the
        // pick.
        // Include continuations: a user who clicked Maks today and
        // resumed three times has only continuation rows for that
        // definition; the head row is whatever they originally
        // named the agent. Picking the most-recent continuation
        // surfaces the same `instance_name` they used last.
        let new_name = match wstore.instance_list_named(
            1,
            Some(&old_def_id),
            /* identity_id */ None,
            /* include_continuations */ true,
        ) {
            Ok(rows) => rows
                .into_iter()
                .next()
                .map(|i| i.instance_name)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| template.name.clone()),
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    error = %e,
                    "template_promote migration: instance_list_named failed; using template name"
                );
                template.name.clone()
            }
        };

        // Idempotency: the migration uses a DETERMINISTIC clone id
        // (`template-promote-v1-<template_id>`) so every retry of
        // every partial-failure scenario targets the same clone.
        // Successful prior steps (zone moves, instance repoints)
        // are reused; failed steps re-attempt against the same
        // destination. There is no way to "fork" the migration
        // into a different clone id, so the unbounded-duplicate
        // failure modes from codex P1 rounds 1+2 cannot recur:
        //
        //   1. Insert def: idempotent via `SELECT WHERE id = ?1`
        //      first; new row only on absence. PK uniqueness on
        //      the deterministic id catches any race.
        //   2. move_zone: write-then-delete; replay copies the
        //      same content to the same destination (no-op when
        //      already moved), retries the source delete.
        //   3. instance_repoint_definition: UPDATE on rows whose
        //      definition_id = old; rows already at new are a
        //      no-op SET.
        //
        // The deterministic id also distinguishes the migration's
        // own clone from any user-created "+ New from template"
        // clone (which lives under a fresh UUID), so we never
        // clobber a user's live session.
        let promote_target_id =
            format!("template-promote-v1-{}", template.id);
        debug_assert!(
            is_valid_definition_id(&promote_target_id),
            "deterministic promote-target id must satisfy the zone-id charset"
        );

        let existing_target = match wstore.agent_def_get(&promote_target_id) {
            Ok(Some(def)) => Some(def),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    promote_target_id = %promote_target_id,
                    error = %e,
                    "template_promote migration: agent_def_get failed; aborting this template"
                );
                stats.failures += 1;
                continue;
            }
        };
        let new_def = if let Some(existing) = existing_target {
            tracing::info!(
                template_id = %old_def_id,
                promote_target_id = %promote_target_id,
                "template_promote migration: reusing prior promote-target clone (idempotent retry)"
            );
            existing
        } else {
            // Clone the template into a new user-owned definition
            // at the deterministic id. Field copies mirror
            // `agent_def_create_from_template`.
            let now = now_ms() as i64;
            // Resolve through the template's own bound bundle when it has
            // one, not the possibly-drifted `db_agent_definitions.provider`
            // column directly (#2594, same pattern as
            // `agent_def_create_from_template`/`forkagentdefinition`). Only
            // `wstore` is available in this migration (no id_store/shared
            // store handoff at this point in the boot sequence) — falls
            // back to `template.provider` when the bundle isn't found via
            // this store, same as it did before this fix.
            let effective_provider = wstore.resolve_effective_provider_id(template);
            let mut new_def = crate::backend::storage::store::AgentDefinition {
                id: promote_target_id.clone(),
                slug: String::new(),
                name: new_name.clone(),
                icon: template.icon.clone(),
                provider: effective_provider,
                description: template.description.clone(),
                working_directory: String::new(),
                shell: template.shell.clone(),
                provider_flags: template.provider_flags.clone(),
                auto_start: 0,
                restart_on_crash: template.restart_on_crash,
                idle_timeout_minutes: template.idle_timeout_minutes,
                created_at: now,
                agent_type: template.agent_type.clone(),
                environment: template.environment.clone(),
                agent_bus_id: String::new(),
                is_seeded: 0,
                accounts: String::new(),
                parent_id: template.id.clone(),
                branch_label: String::new(),
                updated_at: now,
                user_hidden: 0,
                container_image: template.container_image.clone(),
                container_volumes: template.container_volumes.clone(),
                container_name: String::new(),
                use_ambient_login: 0,
                model_vendor_base_url: template.model_vendor_base_url.clone(),
                auto_continue_enabled: 0,
                memory_id: String::new(),
            };
            if let Err(e) = wstore.agent_def_insert(&mut new_def) {
                tracing::warn!(
                    template_id = %old_def_id,
                    promote_target_id = %promote_target_id,
                    error = %e,
                    "template_promote migration: agent_def_insert failed; skipping this template"
                );
                stats.failures += 1;
                continue;
            }
            new_def
        };

        // Move every matching zone (current + archives) onto the new
        // definition id. Per-zone failures are logged but don't abort
        // the whole template — best-effort.
        let mut archives_for_this_def: usize = 0;
        for old_zone in &zones {
            // Build the new zone id by swapping the def-id segment.
            // We know `old_zone` starts with `agent:<old_def_id>:`
            // (per the bucketing above), so substring-replace is safe.
            let suffix = match old_zone.strip_prefix(&format!("agent:{}:", old_def_id)) {
                Some(s) => s,
                None => continue,
            };
            let new_zone = format!("agent:{}:{}", new_def.id, suffix);
            let is_archive = suffix.starts_with("archive:");

            if let Err(e) = move_zone(filestore, old_zone, &new_zone) {
                tracing::warn!(
                    template_id = %old_def_id,
                    old_zone = %old_zone,
                    new_zone = %new_zone,
                    error = %e,
                    "template_promote migration: move_zone failed"
                );
                stats.failures += 1;
                continue;
            }
            if is_archive {
                archives_for_this_def += 1;
            }
        }

        // Repoint any in-DB instances referencing this template at
        // the new user-owned definition. Without this, the existing
        // continueOfInstanceId reattach flow would still look up the
        // template and pass through the un-promoted definition_id.
        let repointed = match wstore.instance_repoint_definition(&old_def_id, &new_def.id) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    template_id = %old_def_id,
                    new_definition_id = %new_def.id,
                    error = %e,
                    "template_promote migration: instance_repoint_definition failed"
                );
                stats.failures += 1;
                0
            }
        };
        stats.instances_repointed += repointed;
        stats.archives_moved += archives_for_this_def;
        stats.templates_promoted += 1;
        tracing::info!(
            template_id = %old_def_id,
            template_name = %template.name,
            new_definition_id = %new_def.id,
            new_name = %new_def.name,
            archives_moved = archives_for_this_def,
            instances_repointed = repointed,
            "template_promote migration: promoted template into user agent"
        );
    }

    // Marker write removed in the 2026-05-24 self-idempotency rework
    // (see doc comment above). The invariant "no seeded def carries a
    // session zone" is checked on every startup; when it already holds
    // this function is a sub-ms no-op.

    tracing::info!(
        templates_scanned = stats.templates_scanned,
        templates_promoted = stats.templates_promoted,
        archives_moved = stats.archives_moved,
        instances_repointed = stats.instances_repointed,
        failures = stats.failures,
        "template_promote migration: complete"
    );

    stats
}

/// Per-file decision inside `move_zone`'s retry-aware loop. See the
/// doc comment in `move_zone` for which round each variant addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyAction {
    /// Destination missing the file (R5 partial-copy fill).
    Copy,
    /// Source strictly newer than destination (R6 newer-source promotion).
    Overwrite,
    /// Destination strictly newer than source (R4 user-continuation
    /// on destination clone) — or equal-modts + equal bytes.
    Preserve,
    /// Equal modts; need to read both sides and compare bytes.
    TieBreakByBytes,
    /// Equal modts but bytes differ — neither side is canonical
    /// (R7 same-ms conflict). Preserve destination, leave source.
    Conflict,
}

/// Move every file in `old_zone` to `new_zone`, preserving names + bytes.
/// Implemented as read-write-delete because FileStore doesn't expose a
/// native rename; the cost is bounded by the per-zone file count (1-2
/// in practice — `output.state.json` + `output`).
fn move_zone(
    filestore: &FileStore,
    old_zone: &str,
    new_zone: &str,
) -> Result<(), String> {
    let files = filestore
        .list_files(old_zone)
        .map_err(|e| format!("list_files: {e}"))?;
    if files.is_empty() {
        return Ok(());
    }
    // Per-file recency-aware copy (codex P1 rounds 4 + 5 + 6 on
    // PR #1017). Three retry shapes need to coexist on the same
    // retry path:
    //
    //   R4 — partial-failure, user continued on the destination
    //        clone (`:current` of the new def). Destination has
    //        NEWER bytes than source. Keep destination; drop
    //        source.
    //   R5 — partial-failure, prior `move_zone` wrote SOME of the
    //        destination files before crashing. Destination has
    //        only some files; the missing ones must be copied
    //        from source. Don't drop source until every source
    //        file has a counterpart at the destination.
    //   R6 — partial-failure, `instance_repoint_definition` was
    //        the step that failed. Instances still point at the
    //        seeded def, user continued — SOURCE bytes are newer
    //        than destination's stale copy. Source must NOT be
    //        dropped without first promoting its newer content
    //        to the destination.
    //
    // Resolve all three via a per-file recency-aware copy:
    //   - destination missing the file → COPY (R5).
    //   - destination has the file, src.modts ≤ dest.modts → keep
    //     destination, no copy (R4).
    //   - destination has the file, src.modts > dest.modts → copy
    //     source over destination (R6).
    // After the loop, every source file has a counterpart at the
    // destination; source can be safely deleted.
    //
    // `modts` ties (or zero on either side) are resolved in favor
    // of keeping the destination, matching the R4 semantics — the
    // common case for a clean first-time retry where both sides
    // hold identical bytes.
    let dest_meta: std::collections::HashMap<String, crate::backend::storage::filestore::WaveFile> = filestore
        .list_files(new_zone)
        .map_err(|e| format!("list_files (new): {e}"))?
        .into_iter()
        .map(|f| (f.name.clone(), f))
        .collect();
    let mut copied = 0usize;
    let mut overwritten = 0usize;
    let mut preserved = 0usize;
    let mut conflicts = 0usize;
    for f in &files {
        let dest = dest_meta.get(&f.name);
        let action = match dest {
            None => CopyAction::Copy, // R5: destination missing
            Some(d) if f.modts > d.modts => CopyAction::Overwrite, // R6
            Some(d) if d.modts > f.modts => CopyAction::Preserve, // R4
            Some(_) => CopyAction::TieBreakByBytes, // R7: equal modts
        };
        let resolved = match action {
            CopyAction::Copy | CopyAction::Overwrite => action,
            CopyAction::Preserve => action,
            CopyAction::Conflict => action, // unreachable from the matcher above; explicit for exhaustiveness
            CopyAction::TieBreakByBytes => {
                // R7 — equal modts (millisecond-granular filestore
                // can write source + destination within the same
                // ms on a real retry). Read both sides and
                // disambiguate by bytes.
                let src_bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                let dest_bytes = filestore
                    .read_file(new_zone, &f.name)
                    .map_err(|e| format!("read_file (dest) {}: {e}", f.name))?
                    .unwrap_or_default();
                if src_bytes == dest_bytes {
                    CopyAction::Preserve
                } else {
                    // Conflict: can't tell which side is canonical.
                    // Preserve destination (matches the round-4
                    // semantics — keep what the user might be
                    // looking at), but refuse to delete source so
                    // the operator (or a future GC pass that can
                    // compare timestamps at a higher resolution)
                    // can resolve. The post-loop missing-files
                    // check would still pass, so we signal the
                    // conflict via a separate counter.
                    CopyAction::Conflict
                }
            }
        };
        match resolved {
            CopyAction::Copy => {
                let bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                write_zone_file(filestore, new_zone, &f.name, &bytes)?;
                copied += 1;
            }
            CopyAction::Overwrite => {
                let bytes = filestore
                    .read_file(old_zone, &f.name)
                    .map_err(|e| format!("read_file {}: {e}", f.name))?
                    .unwrap_or_default();
                write_zone_file(filestore, new_zone, &f.name, &bytes)?;
                overwritten += 1;
            }
            CopyAction::Preserve => {
                preserved += 1;
            }
            CopyAction::Conflict => {
                conflicts += 1;
                tracing::warn!(
                    old_zone = %old_zone,
                    new_zone = %new_zone,
                    file = %f.name,
                    modts = f.modts,
                    "template_promote migration: same-ms conflict — bytes differ at equal modts; preserving destination + leaving source for manual recovery"
                );
            }
            CopyAction::TieBreakByBytes => unreachable!("resolved above"),
        }
    }
    if preserved > 0 || overwritten > 0 || conflicts > 0 {
        tracing::info!(
            old_zone = %old_zone,
            new_zone = %new_zone,
            copied,
            overwritten,
            preserved,
            conflicts,
            "template_promote migration: per-file move (R4 user-continuation, R5 partial-copy fill, R6 newer-source promotion, R7 same-ms conflict)"
        );
    }
    if conflicts > 0 {
        // R7: an equal-modts byte-diff was detected. We don't know
        // which side is canonical, so we preserve both: destination
        // keeps its content, source is left in place for operator
        // / GC recovery. Migration converges next run only if the
        // operator resolves the conflict externally.
        return Ok(());
    }
    // Verify every source file has a counterpart at the
    // destination before dropping source — protects against the
    // R5 partial-write case where write_zone_file silently leaves
    // a file absent at the destination despite returning Ok (no
    // current call path does so, but defending the invariant here
    // is cheap and future-proofs the helper).
    let post_dest: std::collections::HashSet<String> = filestore
        .list_files(new_zone)
        .map_err(|e| format!("list_files (new, post): {e}"))?
        .into_iter()
        .map(|f| f.name)
        .collect();
    let missing: Vec<&str> = files
        .iter()
        .map(|f| f.name.as_str())
        .filter(|n| !post_dest.contains(*n))
        .collect();
    if !missing.is_empty() {
        tracing::warn!(
            old_zone = %old_zone,
            new_zone = %new_zone,
            missing = ?missing,
            "template_promote migration: destination missing files post-copy; leaving source in place for retry"
        );
        return Ok(());
    }
    // Delete the source files only after every write has succeeded.
    // delete_zone wipes the whole zone in one transaction.
    if let Err(e) = filestore.delete_zone(old_zone) {
        // Source delete failure is non-fatal — the new zone has the
        // data; the old zone is now stale duplicate, GC concern.
        tracing::warn!(
            old_zone = %old_zone,
            error = %e,
            "template_promote migration: delete_zone failed after copy; source remains"
        );
    }
    Ok(())
}
