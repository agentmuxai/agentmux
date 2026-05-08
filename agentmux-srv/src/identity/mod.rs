// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity resolution at subprocess spawn time.
//!
//! Issue #678 Phase 2 — turns `(instance.identities[], db_identity_accounts)`
//! into a `HashMap<String, String>` of env vars to merge into the spawn
//! config. Secrets never enter block metadata or the JSON IPC bus; the
//! resolver runs locally in `agentmux-srv` and writes directly into the
//! `Command::env()` map at spawn time.
//!
//! Phase 2 supports two `SecretRef` backends:
//!   - `Env { env_var }`     — read from agentmux-srv's process env
//!   - `PlaintextDev { ... }`— literal value (dev/test only; warns on use)
//!
//! `SecretsManager` (cloud secrets) is reserved for Phase 3 alongside
//! the encrypted vault.

pub mod resolver;

pub use resolver::{
    InjectionEnvVars, ResolveError, build_injection_env, provider_env_vars, resolve_secret,
};

use std::collections::HashMap;

use crate::backend::storage::wstore::{InstanceIdentity, WaveStore};

/// Resolve and merge identity-derived env vars into the spawn env map.
///
/// Looks up the active running instance for `block_id`, falls back to
/// the agent definition's `db_forge_agent_identities` junction if the
/// instance has no identity overrides, then resolves each identity's
/// secret and writes the value into `env_vars`. Identity-derived vars
/// take precedence over anything already in `env_vars` (e.g. a
/// hand-edited `cmd:env` entry — the explicit identity selection
/// always wins).
///
/// Failures are logged but never abort the spawn — the issue spec's
/// "warn, don't block" decision. Callers don't need to handle errors.
pub fn inject_identity_env(
    wstore: &WaveStore,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) {
    let identities = match resolve_identities_for_block(wstore, block_id) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "failed to load identities for block");
            return;
        }
    };

    if identities.is_empty() {
        return;
    }

    let (resolved, errs) = build_injection_env(wstore, &identities);
    for (k, v) in resolved {
        env_vars.insert(k, v);
    }
    for (account_id, err) in errs {
        tracing::warn!(
            block_id = %block_id,
            account_id = %account_id,
            error = %err,
            "identity resolution failed; falling back to ambient credentials for this slot"
        );
    }
}

/// Find the identities to inject for a given block. Per-instance
/// overrides take precedence; falls back to the definition-level
/// `db_forge_agent_identities` junction when the instance has none.
fn resolve_identities_for_block(
    wstore: &WaveStore,
    block_id: &str,
) -> Result<Vec<InstanceIdentity>, String> {
    let instance = match wstore
        .instance_get_active_for_block(block_id)
        .map_err(|e| e.to_string())?
    {
        Some(i) => i,
        None => return Ok(Vec::new()),
    };

    if !instance.identities.is_empty() {
        return Ok(instance.identities);
    }

    // Fall back to the definition-level junction.
    let rows = wstore
        .agent_identity_list_for_agent(&instance.definition_id)
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| InstanceIdentity {
            account_id: r.account_id,
            provider: r.provider,
        })
        .collect())
}
