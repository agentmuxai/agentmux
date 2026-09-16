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
//! this module looks at the latest `db_bundle_versions` row for each
//! manifest id — if `written_by` is this module's own reserved identity
//! (`WRITTEN_BY`), it's safe to overwrite; if it's anything else (an
//! `armory-ui` human edit), the manifest update is skipped and logged
//! rather than clobbering it.

use std::sync::Arc;

use serde::Deserialize;

use super::storage::bundles::Bundle;
use super::storage::bundle_versions::content_hash;
use super::storage::store::Store;
use super::storage::StoreError;

/// The reserved `written_by` identity this seeder stamps on every version it
/// writes. Never used by any other caller — an agent's own write stamps its
/// real `AGENTMUX_AGENT_ID`, a human edit via the Armory UI stamps
/// `"armory-ui"` (`bundle.rs`'s `upsertsystemmemory` handler) — so seeing
/// this exact value on the latest version is what tells `auto_seed_on_
/// startup` that nothing but AgentMux itself has touched the row since.
pub const WRITTEN_BY: &str = "agentmux-operator-config-seed";

const SEED_MANIFEST: &str = include_str!("../../operator-config-seed.json");

#[derive(Debug, Deserialize)]
struct SeedManifest {
    /// Informational/loggable only, same as `agent-seed.json`'s own
    /// `version` field — the reseed decision below is content-hash-based,
    /// not a version-number comparison (see this module's own doc comment).
    version: u32,
    entries: Vec<SeedEntry>,
}

#[derive(Debug, Deserialize)]
struct SeedEntry {
    id: String,
    name: String,
    instructions: String,
}

enum SeedOutcome {
    Created,
    Updated,
    Unchanged,
    SkippedLocalEdit,
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

    let (mut created, mut updated, mut unchanged, mut skipped_local, mut skipped_collision) =
        (0usize, 0usize, 0usize, 0usize, 0usize);

    for entry in &manifest.entries {
        match seed_one(wstore, entry) {
            Ok(SeedOutcome::Created) => created += 1,
            Ok(SeedOutcome::Updated) => updated += 1,
            Ok(SeedOutcome::Unchanged) => unchanged += 1,
            Ok(SeedOutcome::SkippedLocalEdit) => {
                skipped_local += 1;
                tracing::info!(
                    id = %entry.id,
                    "operator config seed: skipping — entry has a local (non-AgentMux) edit, not overwriting"
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

    if created + updated + skipped_local + skipped_collision > 0 {
        tracing::info!(
            "operator config seed: manifest v{}: {created} created, {updated} updated, \
             {unchanged} unchanged, {skipped_local} skipped (local edit), \
             {skipped_collision} skipped (collision)",
            manifest.version,
        );
    }
}

fn seed_one(wstore: &Arc<Store>, entry: &SeedEntry) -> Result<SeedOutcome, StoreError> {
    let history = wstore.bundle_version_list(&entry.id)?;
    let target_hash = content_hash(&entry.name, &entry.instructions);
    let is_new = history.is_empty();

    if let Some(latest) = history.first() {
        if latest.written_by != WRITTEN_BY {
            return Ok(SeedOutcome::SkippedLocalEdit);
        }
        if latest.content_hash == target_hash {
            return Ok(SeedOutcome::Unchanged);
        }
    }

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

    wstore.bundle_upsert_system_with_version(&bundle, WRITTEN_BY, "agentmux_operator_config_seed", "{}")?;
    Ok(if is_new { SeedOutcome::Created } else { SeedOutcome::Updated })
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
        let outcome = seed_one(&store, &e).unwrap();
        assert!(matches!(outcome, SeedOutcome::Created));

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
        seed_one(&store, &e).unwrap();

        let outcome = seed_one(&store, &e).unwrap();
        assert!(matches!(outcome, SeedOutcome::Unchanged));
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 1, "no new version on no-op");
    }

    #[test]
    fn changed_manifest_content_overwrites_when_seeder_owns_it() {
        let store = test_store();
        let e1 = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e1).unwrap();

        let e2 = entry("op-1", "Operator One", "body TWO");
        let outcome = seed_one(&store, &e2).unwrap();
        assert!(matches!(outcome, SeedOutcome::Updated));

        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "body TWO");
        assert_eq!(store.bundle_version_list("op-1").unwrap().len(), 2);
    }

    /// The load-bearing guarantee (spec §3.3, §5 test plan): a human edit
    /// through the Armory UI (`written_by = "armory-ui"`) must survive the
    /// next startup's reseed even though the manifest content differs from
    /// what's stored.
    #[test]
    fn locally_edited_entry_is_never_overwritten() {
        let store = test_store();
        let e1 = entry("op-1", "Operator One", "body one");
        seed_one(&store, &e1).unwrap();

        // Simulate a human editing this system entry via the Armory UI.
        store
            .bundle_version_insert("op-1", "Operator One", "human-edited body", "human", "{}", "armory-ui")
            .unwrap();

        let e2 = entry("op-1", "Operator One", "AgentMux wants to change this");
        let outcome = seed_one(&store, &e2).unwrap();
        assert!(matches!(outcome, SeedOutcome::SkippedLocalEdit));

        // db_bundles itself is untouched by seed_one when skipped — still
        // whatever bundle_upsert_system last wrote (the original seed), the
        // version-only simulated edit above didn't touch db_bundles.
        let bundle = store.bundle_get("op-1").unwrap().unwrap();
        assert_eq!(bundle.instructions, "body one", "seed_one made no db_bundles write on skip");
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
        let result = seed_one(&store, &e);
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
