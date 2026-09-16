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
pub fn auto_seed_on_startup(wstore: &Arc<Store>) {
    let manifest: SeedManifest = match serde_json::from_str(SEED_MANIFEST) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("operator config seed: failed to parse manifest: {e}");
            return;
        }
    };

    let (mut created, mut updated, mut unchanged, mut skipped_local, mut skipped_older, mut skipped_collision) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);

    for entry in &manifest.entries {
        match seed_one(wstore, entry, manifest.version) {
            Ok(BundleReseedOutcome::Created) => created += 1,
            Ok(BundleReseedOutcome::Updated) => updated += 1,
            Ok(BundleReseedOutcome::Unchanged) => unchanged += 1,
            Ok(BundleReseedOutcome::SkippedLocalEdit) => {
                skipped_local += 1;
                tracing::info!(
                    id = %entry.id,
                    "operator config seed: skipping — entry has a local (non-AgentMux) edit, not overwriting"
                );
            }
            Ok(BundleReseedOutcome::SkippedOlderGeneration) => {
                skipped_older += 1;
                tracing::info!(
                    id = %entry.id,
                    manifest_version = manifest.version,
                    "operator config seed: skipping — a newer manifest generation already seeded this row \
                     (this build's own manifest is older; likely a different AgentMux version sharing this store)"
                );
            }
            Err(e) => {
                skipped_collision += 1;
                tracing::warn!(
                    id = %entry.id,
                    name = %entry.name,
                    error = %e,
                    "operator config seed: skipping entry due to upsert error (name collision?)"
                );
            }
        }
    }

    if created + updated + skipped_local + skipped_older + skipped_collision > 0 {
        tracing::info!(
            "operator config seed: manifest v{}: {created} created, {updated} updated, \
             {unchanged} unchanged, {skipped_local} skipped (local edit), \
             {skipped_older} skipped (older generation), {skipped_collision} skipped (collision)",
            manifest.version,
        );
    }
}

fn seed_one(wstore: &Arc<Store>, entry: &SeedEntry, manifest_version: u32) -> Result<BundleReseedOutcome, StoreError> {
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
    wstore.bundle_reseed_system_if_owned(&bundle, WRITTEN_BY, "agentmux_operator_config_seed", &source_detail)
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
