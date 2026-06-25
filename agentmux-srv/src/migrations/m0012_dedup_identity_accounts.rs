// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Deduplicate identity accounts that share the same (provider, credential).
//!
//! Root cause: the old per-channel `backfill_shared_store_once` ran at every
//! channel startup and copied the channel's local account UUID into the shared
//! store. Each channel had a different UUID for the same underlying credential
//! (e.g. the shared claude OAuth dir), so the shared store accumulated one row
//! per channel launch rather than one row per credential.
//!
//! Strategy: group accounts by (provider, secret_ref JSON). For each group of
//! duplicates, keep whichever account is currently referenced by a bundle
//! binding (or the oldest by created_at if none is bound). Reroute any bundle
//! bindings and agent identity links that reference a dupe to the canonical,
//! then delete the dupes — the CASCADE FK handles any remaining joins.

use std::collections::{HashMap, HashSet};

use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0012DedupIdentityAccounts;

impl Migration for M0012DedupIdentityAccounts {
    fn id(&self) -> &'static str { "0012_dedup_identity_accounts" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Deduplicate identity accounts sharing the same provider and credential"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("dedup_identity: open shared store: {}", e)))?;

        let accounts = shared.identity_list(None)
            .map_err(|e| MigrationError(format!("dedup_identity: list accounts: {}", e)))?;

        // Group by (provider, serialized secret_ref). Using JSON serialization
        // gives a stable string key; serde_json output is deterministic for
        // structs with fixed field order.
        let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (i, acct) in accounts.iter().enumerate() {
            let ref_key = serde_json::to_string(&acct.secret_ref)
                .unwrap_or_else(|_| acct.id.clone());
            groups.entry((acct.provider.clone(), ref_key)).or_default().push(i);
        }

        let has_dupes = groups.values().any(|v| v.len() > 1);
        if !has_dupes {
            tracing::info!("dedup_identity_accounts: no duplicates found");
            return Ok(());
        }

        // Gather bundle bindings and agent links before mutating.
        let bundles = shared.bundle_identity_list()
            .map_err(|e| MigrationError(format!("dedup_identity: list bundles: {}", e)))?;

        let mut all_bindings: Vec<(String, String, String)> = Vec::new(); // (bundle_id, provider, account_id)
        for bundle in &bundles {
            let bindings = shared.bundle_identity_bindings(&bundle.id)
                .map_err(|e| MigrationError(format!("dedup_identity: bindings for {}: {}", bundle.id, e)))?;
            for b in bindings {
                all_bindings.push((bundle.id.clone(), b.provider, b.account_id));
            }
        }

        let agent_links = shared.agent_identity_list_all()
            .map_err(|e| MigrationError(format!("dedup_identity: list agent links: {}", e)))?;

        let bound_ids: HashSet<&str> = all_bindings.iter().map(|(_, _, id)| id.as_str()).collect();

        let mut deleted = 0usize;

        for ((provider, _), indices) in &groups {
            if indices.len() <= 1 {
                continue;
            }

            // Sort: bound accounts first, then oldest by created_at.
            let mut sorted: Vec<&_> = indices.iter().map(|&i| &accounts[i]).collect();
            sorted.sort_by(|a, b| {
                let a_bound = bound_ids.contains(a.id.as_str());
                let b_bound = bound_ids.contains(b.id.as_str());
                b_bound.cmp(&a_bound).then_with(|| a.created_at.cmp(&b.created_at))
            });

            let canonical_id = &sorted[0].id;
            let dupe_ids: HashSet<&str> = sorted[1..].iter().map(|a| a.id.as_str()).collect();

            // Rebind any bundle bindings that reference a dupe.
            for (bundle_id, binding_provider, account_id) in &all_bindings {
                if binding_provider == provider && dupe_ids.contains(account_id.as_str()) {
                    shared.bundle_identity_bind(bundle_id, provider, canonical_id)
                        .map_err(|e| MigrationError(format!(
                            "dedup_identity: rebind bundle {} provider {}: {}", bundle_id, provider, e
                        )))?;
                }
            }

            // Relink any agent identity links that reference a dupe.
            for link in &agent_links {
                if &link.provider == provider && dupe_ids.contains(link.account_id.as_str()) {
                    shared.agent_identity_link(&link.agent_id, canonical_id, provider)
                        .map_err(|e| MigrationError(format!(
                            "dedup_identity: relink agent {}: {}", link.agent_id, e
                        )))?;
                }
            }

            // Delete dupes — CASCADE handles any remaining bindings/links.
            for dupe_id in &dupe_ids {
                shared.identity_delete(dupe_id)
                    .map_err(|e| MigrationError(format!("dedup_identity: delete {}: {}", dupe_id, e)))?;
                deleted += 1;
            }
        }

        tracing::info!(deleted, "dedup_identity_accounts: complete");
        Ok(())
    }
}
