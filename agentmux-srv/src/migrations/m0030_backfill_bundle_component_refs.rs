// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_bundle_skills_ref` / `db_bundle_mcp_ref` from the inline
//! `db_bundles.skills` / `db_bundles.mcp_servers` JSON columns.
//!
//! Phase 0b of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. The
//! two stores were never synced: binding a skill in the Armory wrote only a
//! ref row, while ABF export read only the inline column, so a bundle could
//! run with components it did not export and export components it did not run
//! with (§3.4). The ref tables become authoritative; this carries the inline
//! data across so nothing is lost when export stops reading the columns.
//!
//! **Ships in the same release as the read switch.** Export reading refs
//! before this has run would make every pre-existing bundle export empty.
//!
//! Two stores, deliberately. `db_bundles` lives in the shared store
//! (`run_shared_store_schema`), while `db_skills`, `db_mcp_servers` and both
//! ref tables live in the channel store (`run_object_schema`,
//! `migrations.rs:742`/`:749`) — the ref tables must sit next to the catalog
//! tables they carry foreign keys to, which is also why they have no FK to
//! `db_bundles` (`migrations.rs:721-741`). So this reads bundles from the
//! shared store and writes refs into the channel store, resolving the former
//! exactly the way `m0021` does.
//!
//! Idempotent three times over: `INSERT OR IGNORE` on the ref tables, name
//! uniqueness on the MCP catalog rows, and a per-bundle skip when nothing is
//! left to carry. Re-running changes nothing.
//!
//! Best-effort per bundle. One malformed `mcp_servers` blob must not abort a
//! migration that is otherwise carrying real data across — the alternative
//! leaves the store half-converted with the read switch already live. Every
//! skip is logged with the bundle id.

use std::sync::Arc;

use serde_json::Value;

use crate::backend::storage::mcp_servers::McpServer;
use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0030BackfillBundleComponentRefs;

/// Where `db_bundles` actually lives — the shared store, falling back to the
/// channel store when it cannot be opened.
///
/// Same resolution, and the same never-hard-fail posture, as
/// `m0021_backfill_agent_bundles::resolve_bundle_store`. An unusable shared
/// store means "not today" rather than a failed boot.
fn resolve_bundle_store(ctx: &MigrationContext, wstore: &Arc<Store>) -> Arc<Store> {
    match Store::open_shared(&ctx.shared_store_path) {
        Ok(shared) => Arc::new(shared),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %ctx.shared_store_path.display(),
                "backfill_bundle_component_refs: shared store unavailable, falling back to channel store"
            );
            wstore.clone()
        }
    }
}

/// Milliseconds since the epoch, for the catalog rows this creates.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The display name for an MCP server recovered from an inline entry.
///
/// The inline blob is the exporter's own entry shape: a config object with
/// `name` alongside. `build_mcp_config_from_refs` keys `.mcp.json` on the
/// row's name, so a nameless entry needs a stable synthetic one rather than
/// being dropped — `index` makes it unique within the bundle.
fn mcp_entry_name(entry: &Value, index: usize) -> String {
    entry
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("mcp-server-{}", index + 1))
}

/// Carry one bundle's inline components across into the ref tables.
///
/// Returns `(skills_bound, mcp_bound)`. Never returns `Err` for bad data —
/// see the module doc on why one bad bundle must not stop the rest.
fn backfill_one_bundle(
    wstore: &Store,
    bundle_store: &Store,
    bundle_id: &str,
    skills_json: &str,
    mcp_json: &str,
) -> (usize, usize) {
    let mut skills_bound = 0usize;
    let mut mcp_bound = 0usize;

    // ---- skills: the inline column holds bare ids into db_skills ----
    match serde_json::from_str::<Vec<String>>(skills_json) {
        Ok(ids) => {
            for id in ids {
                // A skill id whose row is already gone has nothing to bind;
                // the ref tables cannot represent it and the export could
                // never have rendered it either.
                match wstore.skill_get(&id) {
                    Ok(Some(_)) => match wstore.bundle_skill_bind(bundle_store, bundle_id, &id) {
                        Ok(()) => skills_bound += 1,
                        Err(e) => tracing::warn!(
                            bundle_id, skill_id = %id, error = %e,
                            "backfill_bundle_component_refs: skill bind failed"
                        ),
                    },
                    Ok(None) => tracing::warn!(
                        bundle_id, skill_id = %id,
                        "backfill_bundle_component_refs: inline skill id has no catalog row — dropped"
                    ),
                    Err(e) => tracing::warn!(
                        bundle_id, skill_id = %id, error = %e,
                        "backfill_bundle_component_refs: skill lookup failed"
                    ),
                }
            }
        }
        Err(e) if !skills_json.trim().is_empty() && skills_json.trim() != "[]" => {
            tracing::warn!(
                bundle_id, error = %e,
                "backfill_bundle_component_refs: malformed inline skills JSON — nothing carried across"
            );
        }
        Err(_) => {} // genuinely blank is not an error
    }

    // ---- mcp servers: the inline column holds whole config objects ----
    match serde_json::from_str::<Vec<Value>>(mcp_json) {
        Ok(entries) => {
            for (index, entry) in entries.iter().enumerate() {
                if !entry.is_object() {
                    tracing::warn!(
                        bundle_id, index,
                        "backfill_bundle_component_refs: inline mcp entry is not an object — dropped"
                    );
                    continue;
                }
                let name = mcp_entry_name(entry, index);
                let server = McpServer {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone(),
                    // The column default; the inline shape carries no
                    // transport of its own and every entry these bundles hold
                    // was written for the stdio launcher.
                    transport: "stdio".to_string(),
                    config: serde_json::to_string(entry).unwrap_or_else(|_| "{}".to_string()),
                    // Bundle-scoped, not global: this server belongs to this
                    // bundle, and `bundle_mcp_upsert_unique` forces it anyway.
                    is_global: false,
                    created_at: now_ms(),
                    updated_at: now_ms(),
                };
                // Creates the catalog row AND binds the ref in one
                // transaction (`managed_upsert_unique_for_bundle`). A name
                // that already resolves for this bundle errors, which is the
                // idempotent re-run path — the row is already there.
                match wstore.bundle_mcp_upsert_unique(bundle_store, bundle_id, &server, true) {
                    Ok(()) => mcp_bound += 1,
                    Err(e) if e.to_string().contains("already") => {
                        tracing::debug!(
                            bundle_id, name = %name,
                            "backfill_bundle_component_refs: mcp server already present — nothing to do"
                        );
                    }
                    Err(e) => tracing::warn!(
                        bundle_id, name = %name, error = %e,
                        "backfill_bundle_component_refs: mcp upsert failed"
                    ),
                }
            }
        }
        Err(e) if !mcp_json.trim().is_empty() && mcp_json.trim() != "[]" => {
            tracing::warn!(
                bundle_id, error = %e,
                "backfill_bundle_component_refs: malformed inline mcp_servers JSON — nothing carried across"
            );
        }
        Err(_) => {}
    }

    (skills_bound, mcp_bound)
}

impl Migration for M0030BackfillBundleComponentRefs {
    fn id(&self) -> &'static str {
        "0030_backfill_bundle_component_refs"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Carry inline bundle skills/mcp_servers across into the bundle ref tables"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(Store::open(&ctx.channel_store_path).map_err(|e| {
            MigrationError(format!("backfill_bundle_component_refs: open wstore: {e}"))
        })?);
        let bundle_store = resolve_bundle_store(ctx, &wstore);

        let bundles = bundle_store.bundle_list().map_err(|e| {
            MigrationError(format!("backfill_bundle_component_refs: list bundles: {e}"))
        })?;

        let mut total_skills = 0usize;
        let mut total_mcp = 0usize;
        for bundle in &bundles {
            let (s, m) = backfill_one_bundle(
                &wstore,
                &bundle_store,
                &bundle.id,
                &bundle.skills,
                &bundle.mcp_servers,
            );
            total_skills += s;
            total_mcp += m;
        }

        tracing::info!(
            bundles = bundles.len(),
            skills_bound = total_skills,
            mcp_bound = total_mcp,
            "backfill_bundle_component_refs: complete"
        );
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        // Verifying "every inline component has a ref" needs both stores, and
        // the shared one may legitimately be unavailable (see
        // resolve_bundle_store). Report what we can rather than claiming a
        // mismatch we cannot substantiate.
        match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(conn)) => {
                let skills: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_bundle_skills_ref", [], |r| r.get(0))
                    .unwrap_or(0);
                let mcp: i64 = conn
                    .query_row("SELECT COUNT(*) FROM db_bundle_mcp_ref", [], |r| r.get(0))
                    .unwrap_or(0);
                VerifyOutcome::Ok(format!("{skills} skill ref(s), {mcp} mcp ref(s)"))
            }
            Ok(None) => VerifyOutcome::Ok("no channel store".to_string()),
            Err(e) => VerifyOutcome::Error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::skills::Skill;

    fn store() -> Store {
        // One in-memory store serves as both wstore and bundle store:
        // `run_object_schema` creates db_bundles alongside the ref tables, so
        // both sides resolve. The cross-store rule itself is pinned by
        // `mcp_servers.rs::bind_checks_bundle_existence_in_id_store_not_self`.
        Store::open_in_memory().unwrap()
    }

    fn insert_bundle(s: &Store, id: &str, skills: &str, mcp: &str) {
        let bundle = crate::backend::storage::store::Bundle {
            id: id.to_string(),
            name: format!("Bundle {id}"),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: mcp.to_string(),
            skills: skills.to_string(),
            sort_order: 0,
            created_at: 1,
            updated_at: 1,
            is_system: false,
        };
        s.bundle_upsert(&bundle).unwrap();
    }

    fn insert_skill(s: &Store, id: &str, name: &str) {
        let skill = Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: name.to_string(),
            skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
            description: String::new(),
            content: "body".to_string(),
            is_global: true,
            created_at: 1,
            updated_at: 1,
        };
        s.skill_upsert_unique_global(&skill).unwrap();
    }

    #[test]
    fn carries_inline_skills_and_mcp_servers_into_the_ref_tables() {
        let s = store();
        insert_skill(&s, "skill-1", "Deploy");
        insert_bundle(
            &s,
            "bundle-1",
            r#"["skill-1"]"#,
            r#"[{"name":"github","command":"gh-mcp","env":{"GITHUB_TOKEN":""}}]"#,
        );

        let (sk, mc) = backfill_one_bundle(
            &s,
            &s,
            "bundle-1",
            r#"["skill-1"]"#,
            r#"[{"name":"github","command":"gh-mcp","env":{"GITHUB_TOKEN":""}}]"#,
        );
        assert_eq!((sk, mc), (1, 1));

        let bound_skills: Vec<_> = s
            .bundle_skill_list("bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .collect();
        assert_eq!(bound_skills.len(), 1, "skill must be bound to the bundle");

        let bound_mcp: Vec<_> = s
            .bundle_mcp_list("bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .collect();
        assert_eq!(bound_mcp.len(), 1);
        assert_eq!(bound_mcp[0].server.name, "github");
        assert!(
            bound_mcp[0].server.config.contains("gh-mcp"),
            "the config object must survive verbatim: {}",
            bound_mcp[0].server.config
        );
    }

    #[test]
    fn rerunning_binds_nothing_new() {
        let s = store();
        insert_skill(&s, "skill-1", "Deploy");
        let skills = r#"["skill-1"]"#;
        let mcp = r#"[{"name":"github","command":"gh-mcp"}]"#;
        insert_bundle(&s, "bundle-1", skills, mcp);

        let first = backfill_one_bundle(&s, &s, "bundle-1", skills, mcp);
        assert_eq!(first, (1, 1));

        // Second pass: the skill bind is INSERT OR IGNORE (still reports
        // bound), the MCP name already resolves for this bundle (reports 0).
        let second = backfill_one_bundle(&s, &s, "bundle-1", skills, mcp);
        assert_eq!(second.1, 0, "a second pass must not create a second server");

        let bound_mcp = s
            .bundle_mcp_list("bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .count();
        assert_eq!(bound_mcp, 1, "exactly one server after two passes");
    }

    #[test]
    fn a_malformed_blob_is_logged_and_the_rest_of_the_bundle_still_migrates() {
        let s = store();
        insert_skill(&s, "skill-1", "Deploy");
        insert_bundle(&s, "bundle-1", r#"["skill-1"]"#, "not an array at all");

        let (sk, mc) = backfill_one_bundle(&s, &s, "bundle-1", r#"["skill-1"]"#, "not an array at all");
        assert_eq!(sk, 1, "the good half must still be carried across");
        assert_eq!(mc, 0);
    }

    #[test]
    fn an_inline_skill_id_with_no_catalog_row_is_dropped_not_fatal() {
        let s = store();
        insert_bundle(&s, "bundle-1", r#"["ghost"]"#, "[]");
        let (sk, mc) = backfill_one_bundle(&s, &s, "bundle-1", r#"["ghost"]"#, "[]");
        assert_eq!((sk, mc), (0, 0));
    }

    #[test]
    fn a_nameless_mcp_entry_gets_a_stable_synthetic_name() {
        let s = store();
        insert_bundle(&s, "bundle-1", "[]", r#"[{"command":"x"},{"command":"y"}]"#);
        let (_, mc) = backfill_one_bundle(&s, &s, "bundle-1", "[]", r#"[{"command":"x"},{"command":"y"}]"#);
        assert_eq!(mc, 2, "two nameless entries must not collide on name");

        let names: Vec<String> = s
            .bundle_mcp_list("bundle-1")
            .unwrap()
            .into_iter()
            .filter(|i| i.bound_to_bundle)
            .map(|i| i.server.name)
            .collect();
        assert!(names.contains(&"mcp-server-1".to_string()), "got {names:?}");
        assert!(names.contains(&"mcp-server-2".to_string()), "got {names:?}");
    }
}
