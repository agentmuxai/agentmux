// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Operator Config seed engine: keeps AgentMux's own shipped Global Memory
//! entries (`is_system=1` rows — see docs/specs/SPEC_GLOBAL_MEMORY_
//! SYSTEM_TIER_2026_08_24.md for the isolation, docs/specs/SPEC_SYSTEM_
//! TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md for this module's own design)
//! in sync with a compiled-in manifest, every startup — not a one-time
//! seed like `skill_seed.rs`/`mcp_seed.rs`, because stale Operator Config
//! (describing a since-changed App API) would actively mislead an agent
//! rather than merely being absent.
//!
//! The reseed decision (§3.3 of the spec above) is ownership-based, not a
//! field diff like `agent_seed.rs`'s own `reseed_if_needed`: a human can
//! edit an Operator Config entry today via the unified Armory
//! `MemoryEditor`, and the *entire* value of an entry is its content, so
//! there is no separate "identity field" to diff against safely. Instead
//! `Store::bundle_reseed_system_if_owned` looks at the latest `db_bundle_
//! versions` row for each manifest id — if `written_by` is this module's
//! own reserved identity (`WRITTEN_BY`), it's safe to overwrite; if it's
//! anything else (an `armory-ui` human edit), the manifest update is
//! skipped and logged rather than clobbering it. That check-and-write
//! happens atomically at the `Store` layer, not as two separate calls
//! composed here — see that method's own doc comment (ReAgent P1, PR
//! #3244, a check-then-act race in an earlier version of this module).
//!
//! "In sync" also covers removal: `prune_entries_removed_from_manifest`
//! deletes any seeder-owned row whose manifest entry a later AgentMux
//! release dropped or renamed, so a retired Operator Config note doesn't
//! keep being injected into every future agent forever (Codex P2, PR
//! #3244) — mirroring `agent_seed.rs`'s own "remove a seeded agent not in
//! the manifest" behavior. A human-edited row meets the same fate as
//! everywhere else in this module: left alone, never deleted out from
//! under the human who edited it.

use std::sync::Arc;

use serde::Deserialize;

use super::storage::bundles::Bundle;
use super::storage::store::Store;
use super::storage::{BundleReseedOutcome, StoreError};

/// The reserved `written_by` identity this seeder stamps on every version it
/// writes. Never used by any other caller — an agent's own write stamps its
/// real `AGENTMUX_AGENT_ID`, a human edit via the Armory UI stamps
/// `"armory-ui"` (`bundle.rs`'s `upsertsystemmemory` handler, via `bundle_
/// upsert_system_with_version`) — so seeing this exact value on the latest
/// version is what tells `bundle_reseed_system_if_owned` that nothing but
/// AgentMux itself has touched the row since.
pub const WRITTEN_BY: &str = "agentmux-operator-config-seed";

const SEED_MANIFEST: &str = include_str!("../../operator-config-seed.json");

#[derive(Debug, Deserialize)]
struct SeedManifest {
    /// The primary reseed decision is still content-hash-based, not this
    /// number (see this module's own doc comment) — but `version` also
    /// travels into each write's `source_detail` as a monotonic generation
    /// guard: `bundle_reseed_system_if_owned` refuses to let a write whose
    /// generation is lower than what's already recorded win, so an older
    /// AgentMux build's own (older) manifest can never downgrade a row a
    /// newer build already seeded on a shared store.db (Codex P2, PR #3244).
    version: u32,
    entries: Vec<SeedEntry>,
}

#[derive(Debug, Deserialize)]
struct SeedEntry {
    id: String,
    name: String,
    instructions: String,
}

/// Run Operator Config seeding on startup. Unconditional, every boot, no
/// feature flag — called from `bootstrap.rs` next to `agent_seed`'s own
/// call.
pub fn auto_seed_on_startup(store: &Arc<Store>) {
    let manifest: SeedManifest = match serde_json::from_str(SEED_MANIFEST) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("operator config seed: failed to parse manifest: {e}");
            return;
        }
    };
    seed_from_manifest(store, &manifest);
}

/// The actual seed pass, separated from `auto_seed_on_startup` purely so
/// tests can exercise it against a hand-built `SeedManifest` instead of the
/// embedded JSON.
fn seed_from_manifest(store: &Arc<Store>, manifest: &SeedManifest) {
    // Prune BEFORE seeding, not after: when a release renames a manifest
    // entry's id while keeping the same (UNIQUE) display `name`, the old
    // id's row must be gone before the new id's INSERT is attempted, or
    // that insert fails on the name collision with no retry — leaving the
    // renamed entry absent until the NEXT startup happens to prune the old
    // row first (Codex P2, PR #3244).
    prune_entries_removed_from_manifest(store, manifest);

    let mut tally = SeedTally::default();

    // A collision here can be purely an ordering artifact WITHIN this same
    // manifest, independent of pruning: entry B wants the display name
    // entry A is about to release via its own update (A kept its id, only
    // its `name` changed), but B happens to appear before A in the
    // manifest array, so B's INSERT hits A's still-current name and fails.
    //
    // A single retry pass only resolves a 2-entry swap — a longer
    // dependency CHAIN (C wants a name B is about to release, B wants a
    // name A is about to release, A releases first) can still leave an
    // entry stranded after exactly one retry, depending on which order the
    // chain happens to be walked in. So: keep retrying the still-failing
    // subset until a full pass makes literally zero progress (nothing in
    // it succeeded) — each successful entry in a pass can only unblock
    // entries later in the SAME OR A SUBSEQUENT pass, so a pass with no
    // successes at all means every remaining failure is a real collision
    // (e.g. an unrelated user bundle already owns the name), not an
    // ordering artifact. Bounded by the number of entries: each pass either
    // shrinks the pending set by at least one or terminates the loop.
    // Codex P2, PR #3244.
    let mut pending: Vec<&SeedEntry> = manifest.entries.iter().collect();
    loop {
        let mut still_pending = Vec::new();
        let mut made_progress = false;
        for entry in pending {
            if seed_and_tally(store, entry, manifest.version, &mut tally, false) {
                made_progress = true;
            } else {
                still_pending.push(entry);
            }
        }
        if still_pending.is_empty() {
            break;
        }
        if !made_progress {
            // No entry in this pass succeeded — every remaining failure is
            // a genuine collision, not something a further pass could
            // resolve. Log each as such.
            for entry in still_pending {
                seed_and_tally(store, entry, manifest.version, &mut tally, true);
            }
            break;
        }
        pending = still_pending;
    }

    tally.log(manifest.version);
}

/// Tallies from a startup seed pass, logged once at the end (`log`).
#[derive(Default)]
struct SeedTally {
    created: usize,
    updated: usize,
    unchanged: usize,
    skipped_local: usize,
    skipped_older: usize,
    skipped_collision: usize,
}

impl SeedTally {
    fn log(&self, manifest_version: u32) {
        let Self { created, updated, unchanged, skipped_local, skipped_older, skipped_collision } = *self;
        if created + updated + skipped_local + skipped_older + skipped_collision > 0 {
            tracing::info!(
                "operator config seed: manifest v{manifest_version}: {created} created, {updated} updated, \
                 {unchanged} unchanged, {skipped_local} skipped (local edit), \
                 {skipped_older} skipped (older generation), {skipped_collision} skipped (collision)",
            );
        }
    }
}

/// Seeds one entry and records its outcome in `tally`. Returns `false` only
/// for a collision-type error (`Err`) — the caller's signal to retry once
/// more after the full first pass, in case another entry processed later
/// would have released the colliding name. `is_retry` only affects
/// logging: a first-pass failure is silent (might still resolve), a
/// second-pass failure is a real, logged collision.
fn seed_and_tally(store: &Arc<Store>, entry: &SeedEntry, manifest_version: u32, tally: &mut SeedTally, is_retry: bool) -> bool {
    match seed_one(store, entry, manifest_version) {
        Ok(BundleReseedOutcome::Created) => { tally.created += 1; true }
        Ok(BundleReseedOutcome::Updated) => { tally.updated += 1; true }
        Ok(BundleReseedOutcome::Unchanged) => { tally.unchanged += 1; true }
        Ok(BundleReseedOutcome::SkippedLocalEdit) => {
            tally.skipped_local += 1;
            tracing::info!(
                id = %entry.id,
                "operator config seed: skipping — entry has a local (non-AgentMux) edit, not overwriting"
            );
            true
        }
        Ok(BundleReseedOutcome::SkippedOlderGeneration) => {
            tally.skipped_older += 1;
            tracing::info!(
                id = %entry.id,
                manifest_version,
                "operator config seed: skipping — a newer manifest generation already seeded this row \
                 (this build's own manifest is older; likely a different AgentMux version sharing this store)"
            );
            true
        }
        Ok(BundleReseedOutcome::SkippedRetired) => {
            tally.skipped_older += 1;
            tracing::info!(
                id = %entry.id,
                manifest_version,
                "operator config seed: skipping — a newer manifest generation already retired this id \
                 (this build's own manifest still lists it; not resurrecting it)"
            );
            true
        }
        Err(e) => {
            if is_retry {
                tally.skipped_collision += 1;
                tracing::warn!(
                    id = %entry.id,
                    name = %entry.name,
                    error = %e,
                    "operator config seed: skipping entry due to upsert error (name collision?)"
                );
            }
            false
        }
    }
}

/// A row for an entry AgentMux itself removed (or renamed to a new id) in a
/// later release must not linger as a seeder-owned `is_system` row forever —
/// it would keep being injected into every future agent indefinitely,
/// exactly the "stale Operator Config actively misleads an agent" failure
/// mode this whole seeding mechanism exists to prevent (Codex P2, PR #3244).
/// Only prunes rows still owned by this seeder AND whose own recorded
/// generation is <= this process's manifest version (`bundle_delete_system_
/// if_owned` checks both atomically) — on a machine sharing store.db across
/// multiple AgentMux builds/versions, an OLDER build's manifest simply
/// doesn't mention an entry a NEWER build already added; without the
/// generation gate, the older build would misread "not in my manifest" as
/// "removed" and delete the newer build's own addition. Same "never clobber
/// a local edit" posture as the reseed path for a human-edited row: left in
/// place, orphaned from the manifest but not clobbered, for a human to
/// clean up. Codex P2, PR #3244.
fn prune_entries_removed_from_manifest(store: &Arc<Store>, manifest: &SeedManifest) {
    let manifest_ids: std::collections::HashSet<&str> =
        manifest.entries.iter().map(|e| e.id.as_str()).collect();

    let existing_ids = match store.bundle_list_system_ids() {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!("operator config seed: failed to list existing system entries for pruning: {e}");
            return;
        }
    };

    let mut pruned = 0usize;
    for id in existing_ids {
        if manifest_ids.contains(id.as_str()) {
            continue;
        }
        match store.bundle_delete_system_if_owned(&id, WRITTEN_BY, manifest.version) {
            Ok(true) => {
                pruned += 1;
                tracing::info!(id = %id, "operator config seed: pruned an entry no longer in the manifest");
            }
            Ok(false) => {
                tracing::info!(
                    id = %id,
                    "operator config seed: an entry is no longer in this manifest but was left in place \
                     (either it has a local edit, or its own generation is newer than this manifest's — \
                     likely a different, newer AgentMux build sharing this store)"
                );
            }
            Err(e) => {
                tracing::warn!(id = %id, error = %e, "operator config seed: failed to prune an entry no longer in the manifest");
            }
        }
    }
    if pruned > 0 {
        tracing::info!("operator config seed: pruned {pruned} entries no longer in the manifest");
    }
}

fn seed_one(store: &Arc<Store>, entry: &SeedEntry, manifest_version: u32) -> Result<BundleReseedOutcome, StoreError> {
    let now = agentmux_common::time::now_ms();
    let bundle = Bundle {
        id: entry.id.clone(),
        name: entry.name.clone(),
        description: String::new(),
        is_blank: false,
        is_global: true,
        provider: String::new(),
        model: String::new(),
        instructions: entry.instructions.clone(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        created_at: now,
        updated_at: now,
        is_system: true,
    };

    // manifest_version travels in source_detail so bundle_reseed_system_if_
    // owned can refuse to let an older AgentMux build's own (older) manifest
    // downgrade a newer build's already-seeded content on a shared store.db
    // (Codex P2, PR #3244).
    let source_detail = format!(r#"{{"manifest_version":{manifest_version}}}"#);
    store.bundle_reseed_system_if_owned(&bundle, WRITTEN_BY, "agentmux_operator_config_seed", &source_detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Arc<Store> {
        Arc::new(Store::open_in_memory().unwrap())
    }

    fn entry(id: &str, name: &str, instructions: &str) -> SeedEntry {
        SeedEntry {
            id: id.to_string(),
            name: name.to_string(),
            instructions: instructions.to_string(),
        }
    }

    #[test]
    fn embedded_manifest_parses() {
        let manifest: SeedManifest = serde_json::from_str(SEED_MANIFEST).expect("valid JSON");
        assert!(!manifest.entries.is_empty());
        for e in &manifest.entries {
            assert!(!e.id.is_empty());
            assert!(!e.name.is_empty());
            assert!(!e.instructions.trim().is_empty());
        }
    }

    #[test]
    fn fresh_install_creates_every_entry() {
        let store = test_store();
        let e = entry("op-1", "Operator One", "body one");
        let outcome = seed_one(&store, &e, 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Created));

        let bundle = store.bundle_get("op-1").unwrap().expect("row exists");
        assert!(bundle.is_system);
        assert!(bundle.is_global);
        assert_eq!(bundle.instructions, "body one");

        let history = store.bundle_version_list("op-1").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].written_by, WRITTEN_BY);
    }

    #[test]
    fn unchanged_manifest_is_a_no_op() {
        let store = test_store();
        let e = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e, 1).unwrap();

        let outcome = seed_one(&store, &e, 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Unchanged));
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 1, "no new version on no-op");
    }

    /// Codex P2, PR #3244: content being unchanged must not mean the
    /// recorded generation stays stale — a newer manifest generation whose
    /// text happens to match what's already stored (e.g. a revert to an
    /// earlier generation's exact wording) still needs to record that a
    /// newer generation has now confirmed this content, or a concurrently
    /// running build on a generation in between could later pass the
    /// generation check against the stale recorded value and downgrade it.
    #[test]
    fn unchanged_content_still_advances_the_recorded_generation() {
        let store = test_store();
        seed_one(&store, &entry("op-1", "Operator One", "the text"), 1).unwrap();

        // Generation 3's manifest text happens to be identical to
        // generation 1's (a revert).
        let outcome = seed_one(&store, &entry("op-1", "Operator One", "the text"), 3).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Unchanged), "content itself really is unchanged");
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 2, "the generation bump must still be recorded as a version");

        // A generation-2 build, with its own different text, must not be
        // able to downgrade generation 3's already-confirmed content just
        // because 2 > the ORIGINAL (now-stale) recorded generation of 1.
        let outcome = seed_one(&store, &entry("op-1", "Operator One", "generation 2's own different text"), 2).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedOlderGeneration));
        assert_eq!(store.bundle_get("op-1").unwrap().unwrap().instructions, "the text");
    }

    #[test]
    fn changed_manifest_content_overwrites_when_seeder_owns_it() {
        let store = test_store();
        let e1 = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e1, 1).unwrap();

        let e2 = entry("op-1", "Operator One", "body TWO");
        let outcome = seed_one(&store, &e2, 2).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Updated));

        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "body TWO");
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 2);
    }

    /// Codex P2, PR #3244: a compatible-schema OLDER build sharing this
    /// store might still run a pre-#3244 `upsertsystemmemory` handler that
    /// calls the unversioned `bundle_upsert_system` directly — changing the
    /// live row's content without ever touching `db_bundle_versions`. The
    /// latest version would then (wrongly) still show the seeder as the
    /// last writer. seed_one must notice the live content no longer
    /// matches what that version claims, and refuse to overwrite it as if
    /// nothing had happened.
    #[test]
    fn unrecorded_edit_from_an_unversioned_writer_is_never_overwritten() {
        let store = test_store();
        seed_one(&store, &entry("op-1", "Operator One", "seeded body"), 1).unwrap();

        // Simulate an older build's unversioned write path: bundle_upsert_
        // system changes db_bundles directly, recording no version at all.
        let mut edited = store.bundle_get("op-1").unwrap().unwrap();
        edited.instructions = "edited by an older, unversioned build".to_string();
        store.bundle_upsert_system(&edited).unwrap();

        // db_bundle_versions still shows the seeder as the last writer —
        // the whole point of this test.
        let history = store.bundle_version_list("op-1").unwrap();
        assert_eq!(history.len(), 1, "the unversioned write left no trace in the version table");
        assert_eq!(history[0].written_by, WRITTEN_BY);

        let outcome = seed_one(&store, &entry("op-1", "Operator One", "a real manifest update"), 2).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedLocalEdit));

        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "edited by an older, unversioned build", "the unrecorded edit must survive");
    }

    /// ReAgent P2, PR #3244: a row that exists but has ZERO version history
    /// at all (possible via the still-present unversioned bundle_upsert_
    /// system, with no prior seed_one call to have created one) must be
    /// left alone by seed_one, same conservative policy bundle_delete_
    /// system_if_owned already implements for the identical edge case —
    /// not silently overwritten by falling through to the unconditional
    /// upsert.
    #[test]
    fn row_with_zero_version_history_is_never_overwritten() {
        let store = test_store();
        // Created via the plain, unversioned path directly — no seed_one
        // call precedes this, so db_bundle_versions has nothing for "op-1".
        let bundle = Bundle {
            id: "op-1".to_string(),
            name: "Operator One".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: true,
            provider: String::new(),
            model: String::new(),
            instructions: "content with no version history".to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: true,
        };
        store.bundle_upsert_system(&bundle).unwrap();
        assert!(store.bundle_version_list("op-1").unwrap().is_empty(), "precondition: no version history at all");

        let outcome = seed_one(&store, &entry("op-1", "Operator One", "a manifest update"), 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedLocalEdit));
        assert_eq!(store.bundle_get("op-1").unwrap().unwrap().instructions, "content with no version history");
    }

    /// Codex P2, PR #3244: two AgentMux builds sharing store.db (a supported
    /// configuration — see "Multiple Instances Run in Parallel" in this
    /// repo's own CLAUDE.md) must not let an older build's own older
    /// manifest downgrade a row a newer build already seeded, even though
    /// both share the same `written_by` seeder identity.
    #[test]
    fn older_manifest_generation_never_downgrades_a_newer_seed() {
        let store = test_store();
        let e_v3 = entry("op-1", "Operator One", "content from generation 3");
        seed_one(&store, &e_v3, 3).unwrap();

        // A different (older) AgentMux build, running its own older
        // manifest (generation 1), starts up and reseeds the same row.
        let e_v1 = entry("op-1", "Operator One", "content from generation 1");
        let outcome = seed_one(&store, &e_v1, 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedOlderGeneration));

        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "content from generation 3", "newer generation's content must survive");
    }

    /// The load-bearing guarantee (spec §3.3, §5 test plan): a human edit
    /// through the Armory UI must survive the next startup's reseed even
    /// though the manifest content differs from what's stored. Simulated via
    /// `bundle_upsert_system_with_version(..., "armory-ui", ...)` — the exact
    /// call `upsertsystemmemory` (`bundle.rs`) now makes for a real UI save —
    /// not a raw `bundle_version_insert`, so this test actually exercises the
    /// bug ReAgent/Codex flagged on PR #3244: an earlier revision of the real
    /// handler called the non-versioned `bundle_upsert_system`, which left no
    /// version behind at all, so the ownership check never saw the edit and
    /// would have silently overwritten it on the next reseed.
    #[test]
    fn locally_edited_entry_is_never_overwritten() {
        let store = test_store();
        let e1 = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e1, 1).unwrap();

        // Simulate a human editing this system entry via the real Armory UI
        // save path (`upsertsystemmemory` -> `bundle_upsert_system_with_version`).
        let edited = Bundle {
            id: "op-1".to_string(),
            name: "Operator One".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: true,
            provider: String::new(),
            model: String::new(),
            instructions: "human-edited body".to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: true,
        };
        store.bundle_upsert_system_with_version(&edited, "armory-ui", "human", "{}").unwrap();

        let e2 = entry("op-1", "Operator One", "AgentMux wants to change this");
        let outcome = seed_one(&store, &e2, 2).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedLocalEdit));

        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "human-edited body", "seed_one made no db_bundles write on skip");
    }

    /// Codex P2, PR #3244: an entry AgentMux itself removes from a later
    /// manifest (renamed id, retired content, ...) must not linger forever
    /// as a seeder-owned row, still injected into every future agent.
    #[test]
    fn seeder_owned_entry_removed_from_manifest_is_pruned() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();
        assert!(store.bundle_get("op-old").unwrap().is_some());

        // This release's manifest no longer mentions "op-old" at all.
        let manifest = SeedManifest { version: 2, entries: vec![entry("op-new", "New Entry", "new content")] };
        // seed_one for the still-present entry, then run the prune pass
        // directly (mirrors what auto_seed_on_startup does after its loop).
        seed_one(&store, &manifest.entries[0], manifest.version).unwrap();
        prune_entries_removed_from_manifest(&store, &manifest);

        assert!(store.bundle_get("op-old").unwrap().is_none(), "seeder-owned, manifest-removed entry must be pruned");
        assert!(store.bundle_get("op-new").unwrap().is_some(), "entries still in the manifest are untouched");
    }

    /// ReAgent P1, PR #3244: the delete-side mirror of `unrecorded_edit_
    /// from_an_unversioned_writer_is_never_overwritten` — a compatible-
    /// schema OLDER build's unversioned write leaves `written_by` stale at
    /// the seeder's identity in `db_bundle_versions`, even though the live
    /// row is now a human's edit. Pruning must notice the live content
    /// doesn't match what that stale version claims and refuse to delete —
    /// deleting an unrecorded edit is strictly worse than overwriting one.
    #[test]
    fn prune_does_not_delete_an_unrecorded_edit_from_an_unversioned_writer() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "seeded content"), 1).unwrap();

        // Simulate an older build's unversioned write path.
        let mut edited = store.bundle_get("op-old").unwrap().unwrap();
        edited.instructions = "edited by an older, unversioned build".to_string();
        store.bundle_upsert_system(&edited).unwrap();

        // This release's manifest no longer mentions "op-old" — an
        // ordinary prune would otherwise delete it outright.
        let manifest = SeedManifest { version: 2, entries: vec![] };
        prune_entries_removed_from_manifest(&store, &manifest);

        let bundle = store.bundle_get("op-old").unwrap();
        assert!(bundle.is_some(), "an unrecorded edit must never be deleted");
        assert_eq!(bundle.unwrap().instructions, "edited by an older, unversioned build");
    }

    /// The same "never clobber a local edit" guarantee applies to pruning:
    /// a human-edited entry that AgentMux later removes from its manifest
    /// must be left in place, not silently deleted out from under them.
    #[test]
    fn locally_edited_entry_removed_from_manifest_is_not_pruned() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();

        let edited = {
            let mut b = store.bundle_get("op-old").unwrap().unwrap();
            b.instructions = "a human edit worth keeping".to_string();
            b
        };
        store.bundle_upsert_system_with_version(&edited, "armory-ui", "human", "{}").unwrap();

        let manifest = SeedManifest { version: 2, entries: vec![] };
        prune_entries_removed_from_manifest(&store, &manifest);

        let bundle = store.bundle_get("op-old").unwrap();
        assert!(bundle.is_some(), "a human-edited entry must survive even after its manifest entry is removed");
        assert_eq!(bundle.unwrap().instructions, "a human edit worth keeping");
    }

    /// ReAgent/Codex P2, PR #3244: pruning deliberately retires a row — an
    /// OLDER build's own manifest (which still lists the retired id) must
    /// not resurrect it just because the row is gone. Without the
    /// retirement tombstone, "no db_bundles row" reads exactly like "never
    /// existed," which is precisely the bug this test guards against.
    #[test]
    fn seed_one_does_not_resurrect_an_entry_retired_by_a_newer_generation() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();

        // A newer build (generation 5) prunes it — its own manifest no
        // longer lists "op-old" at all.
        assert!(store.bundle_delete_system_if_owned("op-old", WRITTEN_BY, 5).unwrap());
        assert!(store.bundle_get("op-old").unwrap().is_none());

        // An older build, still on generation 3, whose OWN manifest still
        // lists "op-old", starts up and tries to seed it.
        let outcome = seed_one(&store, &entry("op-old", "Old Entry", "old content"), 3).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::SkippedRetired));
        assert!(store.bundle_get("op-old").unwrap().is_none(), "a retired entry must not be resurrected by an older manifest");
    }

    /// The mirror case: once this process's own manifest generation has
    /// caught up to (or passed) the retirement generation, and its own
    /// manifest legitimately re-adds the id, creation must proceed
    /// normally — a tombstone blocks resurrection by a STALE manifest, not
    /// a deliberate re-introduction by a manifest that's caught up.
    #[test]
    fn seed_one_recreates_a_retired_entry_once_caller_generation_catches_up() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();
        assert!(store.bundle_delete_system_if_owned("op-old", WRITTEN_BY, 5).unwrap());

        let outcome = seed_one(&store, &entry("op-old", "Reintroduced Entry", "new content"), 5).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Created));
        assert_eq!(store.bundle_get("op-old").unwrap().unwrap().instructions, "new content");
    }

    /// An ORDINARY human delete via the Armory UI (`bundle_delete_system`,
    /// via `deletesystemmemory` — records no version at all) must not leave
    /// a tombstone behind; a manually deleted entry still comes back
    /// exactly as before this fix (`deleted_entry_is_recreated_...` below).
    /// Only a generation-aware prune (`bundle_delete_system_if_owned`)
    /// leaves a tombstone.
    #[test]
    fn seed_one_still_recreates_after_an_ordinary_human_delete_no_tombstone() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();
        assert!(store.bundle_delete_system("op-old").unwrap());

        let outcome = seed_one(&store, &entry("op-old", "Old Entry", "old content"), 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Created));
    }

    /// Codex P2, PR #3244: on a machine sharing store.db across two
    /// AgentMux builds, an OLDER build's prune pass must not delete an
    /// entry a NEWER build already added just because the older build's
    /// own manifest doesn't mention it yet — "not in my manifest" only
    /// means "removed" when this process's own generation has caught up to
    /// (or passed) the row's.
    #[test]
    fn prune_does_not_delete_an_entry_from_a_newer_generation_it_does_not_know_about() {
        let store = test_store();
        // A newer AgentMux build (generation 5) seeds an entry its own
        // manifest introduced.
        seed_one(&store, &entry("op-new", "New Entry", "added in gen 5"), 5).unwrap();

        // An older build, still on generation 3, starts up. Its own
        // manifest has no idea "op-new" exists.
        let older_manifest = SeedManifest { version: 3, entries: vec![] };
        prune_entries_removed_from_manifest(&store, &older_manifest);

        assert!(
            store.bundle_get("op-new").unwrap().is_some(),
            "an older build must not delete a newer build's own addition"
        );
    }

    /// The mirror-image case: once a build's own manifest generation has
    /// genuinely caught up to (or passed) an entry's generation, and that
    /// entry is still absent from its manifest, it really was removed —
    /// pruning must proceed.
    #[test]
    fn prune_still_deletes_once_this_process_generation_has_caught_up() {
        let store = test_store();
        seed_one(&store, &entry("op-old", "Old Entry", "added in gen 1"), 1).unwrap();

        let later_manifest = SeedManifest { version: 3, entries: vec![] };
        prune_entries_removed_from_manifest(&store, &later_manifest);

        assert!(store.bundle_get("op-old").unwrap().is_none(), "a genuinely-removed entry must still be pruned");
    }

    /// Codex P2, PR #3244: `bundle_delete_system` removes only `db_bundles`,
    /// leaving the append-only version history behind. A reseed must notice
    /// the row itself is gone and recreate it, not take the "unchanged"
    /// shortcut just because the stale history's content hash still matches.
    #[test]
    fn deleted_entry_is_recreated_even_when_manifest_content_is_unchanged() {
        let store = test_store();
        let e = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e, 1).unwrap();
        assert!(store.bundle_delete_system("op-1").unwrap(), "row deleted");
        assert!(store.bundle_get("op-1").unwrap().is_none());
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 1, "history survives the delete");

        let outcome = seed_one(&store, &e, 1).unwrap();
        assert!(matches!(outcome, BundleReseedOutcome::Created), "row existence, not history, gates recreation");
        assert!(store.bundle_get("op-1").unwrap().is_some());
    }

    #[test]
    fn name_collision_with_existing_ordinary_bundle_is_skipped_not_fatal() {
        let store = test_store();
        // An ordinary (non-system) bundle already owns this name — db_bundles.name is UNIQUE table-wide.
        let existing = Bundle {
            id: "user-bundle-1".to_string(),
            name: "Operator One".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: "unrelated user content".to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        };
        store.bundle_upsert(&existing).unwrap();

        let e = entry("op-1", "Operator One", "body one");
        let result = seed_one(&store, &e, 1);
        assert!(result.is_err(), "name UNIQUE constraint should surface as an error, not silently create op-1");
        assert!(store.bundle_get("op-1").unwrap().is_none());
    }

    /// Codex P2, PR #3244: entry B (a brand-new id) wants the display name
    /// entry A (an existing id, unchanged) is about to release via its own
    /// update in this SAME manifest — but B appears first in manifest
    /// order, so its INSERT hits A's still-current name on the first pass.
    /// The retry pass must resolve this without either entry ending up
    /// missing, independent of manifest array order.
    #[test]
    fn seeding_succeeds_regardless_of_entry_order_within_one_manifest() {
        let store = test_store();
        seed_one(&store, &entry("op-a", "Name X", "original A content"), 1).unwrap();

        // Generation 2: A keeps its id but its name changes to "Name Y",
        // freeing up "Name X" for the brand-new "op-b". B is listed BEFORE
        // A in this manifest — the ordering this fix must not depend on.
        let manifest = SeedManifest {
            version: 2,
            entries: vec![
                entry("op-b", "Name X", "B content"),
                entry("op-a", "Name Y", "updated A content"),
            ],
        };
        seed_from_manifest(&store, &manifest);

        let a = store.bundle_get("op-a").unwrap().expect("A must still exist");
        assert_eq!(a.name, "Name Y");
        assert_eq!(a.instructions, "updated A content");

        let b = store.bundle_get("op-b").unwrap().expect("B must exist despite the first-pass ordering collision");
        assert_eq!(b.name, "Name X");
        assert_eq!(b.instructions, "B content");
    }

    /// Codex P2, PR #3244: a single retry pass only resolves a 2-entry
    /// name swap — a 3-entry dependency CHAIN (C wants B's name, B wants
    /// A's name, A releases first) needs the retry to repeat until a full
    /// pass makes no further progress, or the entry at the far end of the
    /// chain (C here) is stranded until the next startup.
    #[test]
    fn seeding_resolves_a_three_entry_name_dependency_chain() {
        let store = test_store();
        seed_one(&store, &entry("op-a", "Name A", "original A"), 1).unwrap();
        seed_one(&store, &entry("op-b", "Name B", "original B"), 1).unwrap();

        // Worst-case order: C (wants B's name) and B (wants A's name) both
        // appear BEFORE A (the one entry that can succeed outright on pass 1).
        let manifest = SeedManifest {
            version: 2,
            entries: vec![
                entry("op-c", "Name B", "C content"),
                entry("op-b", "Name A", "updated B"),
                entry("op-a", "Name C", "updated A"),
            ],
        };
        seed_from_manifest(&store, &manifest);

        assert_eq!(store.bundle_get("op-a").unwrap().unwrap().name, "Name C");
        assert_eq!(store.bundle_get("op-b").unwrap().unwrap().name, "Name A");
        let c = store.bundle_get("op-c").unwrap().expect("C must exist despite being at the end of the dependency chain");
        assert_eq!(c.name, "Name B");
        assert_eq!(c.instructions, "C content");
    }

    #[test]
    fn auto_seed_on_startup_does_not_panic_on_real_manifest() {
        // Smoke test: the real embedded manifest seeds cleanly into an
        // empty store end-to-end (parse -> seed_one -> db writes).
        let store = test_store();
        auto_seed_on_startup(&store);
        let manifest: SeedManifest = serde_json::from_str(SEED_MANIFEST).unwrap();
        for e in &manifest.entries {
            let bundle = store.bundle_get(&e.id).unwrap().expect("seeded");
            assert!(bundle.is_system);
        }
    }
}
