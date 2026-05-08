// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity → env-var resolver.
//!
//! Per-provider matrix of which env vars carry which credential. The
//! GitHub PAT becomes both `GITHUB_TOKEN` and `GH_TOKEN` because both
//! the official `gh` CLI and direct API consumers (curl, oct.js) read
//! one or the other; emitting both is the lowest-friction way to make
//! every common workflow Just Work.

use std::collections::HashMap;
use std::sync::Arc;

use crate::backend::storage::error::StoreError;
use crate::backend::storage::wstore::{SecretRef, WaveStore};

/// Errors specific to the resolver. Every variant is recoverable
/// (the spawn proceeds with whatever env vars resolved successfully)
/// — they exist for tracing visibility, not control flow.
#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("account not found: {0}")]
    AccountNotFound(String),

    #[error("env var not set in srv environment: {0}")]
    EnvVarMissing(String),

    #[error("AWS Secrets Manager backend not yet supported (Phase 3)")]
    SecretsManagerUnsupported,

    #[error("PlaintextDev secrets are disabled in release builds")]
    PlaintextDevDisabledInRelease,

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

/// Map a `(provider, secret-string)` pair to the env vars that should
/// receive it. Returning a Vec lets a single secret populate multiple
/// var names — github writes both GITHUB_TOKEN and GH_TOKEN, AWS
/// writes its standard triplet (today only the access-key-id slot is
/// expressed; multi-secret AWS modeled as three separate accounts is
/// the documented workaround until the matrix learns multi-var
/// emission).
pub fn provider_env_vars(provider: &str) -> Vec<&'static str> {
    match provider {
        "github" => vec!["GITHUB_TOKEN", "GH_TOKEN"],
        "anthropic" => vec!["ANTHROPIC_API_KEY"],
        "openai" => vec!["OPENAI_API_KEY"],
        "kimi" => vec!["MOONSHOT_API_KEY"],
        "aws" => vec!["AWS_ACCESS_KEY_ID"],
        _ => Vec::new(),
    }
}

/// Resolve a `SecretRef` to the plaintext credential string. Each
/// backend has a distinct path:
///
/// - **Env**: read `env_var` from the srv process's own environment.
///   Caller is expected to have set this in their shell or a
///   .env-style loader before launching AgentMux.
/// - **PlaintextDev**: return the literal stored string. **Debug
///   builds only** — guarded behind `cfg(debug_assertions)`. In
///   release builds, the same call returns
///   [`ResolverError::PlaintextDevDisabledInRelease`] so a forgotten
///   dev-secret never leaks into a packaged binary. Reagent P1 on
///   PR #751 caught the missing guard. Phase 3's encrypted vault is
///   the production path.
/// - **SecretsManager**: deferred. Returns
///   [`ResolverError::SecretsManagerUnsupported`].
pub fn resolve_secret(secret_ref: &SecretRef) -> Result<String, ResolverError> {
    match secret_ref {
        SecretRef::Env { env_var } => std::env::var(env_var)
            .map_err(|_| ResolverError::EnvVarMissing(env_var.clone())),
        SecretRef::PlaintextDev { plaintext_dev } => {
            #[cfg(debug_assertions)]
            {
                Ok(plaintext_dev.clone())
            }
            #[cfg(not(debug_assertions))]
            {
                let _ = plaintext_dev;
                Err(ResolverError::PlaintextDevDisabledInRelease)
            }
        }
        SecretRef::SecretsManager { .. } => Err(ResolverError::SecretsManagerUnsupported),
    }
}

/// Inject identity-derived env vars into the spawn map for a block.
///
/// This is the public entry point called from the CLI-spawn paths
/// (`AgentInputCommand` in websocket.rs and `AgentSendCommand` in
/// app_api.rs). Resolution flow:
///
/// 1. Look up the active `AgentInstance` for this block. If none
///    exists, the caller didn't go through the launch modal — return
///    immediately, no injection.
/// 2. Read its `identity_id`. Empty / "blank" → no injection (the
///    user picked the blank singleton at launch, meaning "use ambient
///    creds").
/// 3. Read the `db_identity_bindings` rows for that identity_id.
/// 4. For each binding: fetch the account, resolve its `SecretRef`,
///    look up the provider's env-var matrix, write each var into
///    `env_vars`. Any per-binding failure is logged and skipped —
///    other bindings still inject. The agent CLI launches with
///    whatever resolved cleanly plus whatever ambient env was already
///    in the spawn map.
///
/// This function is intentionally infallible at the top level. It
/// has no `Result`, just side-effects on `env_vars` and `tracing::warn`
/// for every per-binding error. The spawn never aborts because a
/// secret didn't resolve.
pub fn inject_identity_env(
    wstore: Arc<WaveStore>,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) {
    // Step 1: instance lookup.
    let instance = match wstore.instance_get_active_for_block(block_id) {
        Ok(Some(i)) => i,
        Ok(None) => {
            // Block has no agent instance row — nothing to inject.
            return;
        }
        Err(e) => {
            tracing::warn!(target: "identity", "instance lookup failed for block {}: {}", block_id, e);
            return;
        }
    };

    // Step 2: identity_id check.
    if instance.identity_id.is_empty() || instance.identity_id == "blank" {
        // Blank singleton or unset → ambient creds.
        return;
    }

    // Step 3: bindings.
    let bindings = match wstore.bundle_identity_bindings(&instance.identity_id) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "identity",
                "bindings lookup failed for identity {}: {}",
                instance.identity_id,
                e,
            );
            return;
        }
    };

    if bindings.is_empty() {
        // Identity exists but has no accounts bound. Nothing to inject.
        return;
    }

    // Step 4: per-binding resolution + env-var write.
    for binding in &bindings {
        let env_keys = provider_env_vars(&binding.provider);
        if env_keys.is_empty() {
            tracing::warn!(
                target: "identity",
                "no env-var mapping for provider {} (binding for identity {})",
                binding.provider,
                instance.identity_id,
            );
            continue;
        }

        let account = match wstore.identity_get(&binding.account_id) {
            Ok(Some(a)) => a,
            Ok(None) => {
                tracing::warn!(
                    target: "identity",
                    "account {} bound to identity {} but row not found — skipping",
                    binding.account_id,
                    instance.identity_id,
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    "account lookup failed for {}: {}",
                    binding.account_id,
                    e,
                );
                continue;
            }
        };

        let secret = match resolve_secret(&account.secret_ref) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    "secret resolution failed for account {} (provider {}): {} — skipping",
                    binding.account_id,
                    binding.provider,
                    e,
                );
                continue;
            }
        };

        for key in env_keys {
            env_vars.insert(key.to_string(), secret.clone());
        }

        tracing::info!(
            target: "identity",
            "injected {} env var(s) for provider {} (identity={}, account={})",
            provider_env_vars(&binding.provider).len(),
            binding.provider,
            instance.identity_id,
            binding.account_id,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::wstore::{
        AgentInstance, Identity, IdentityAccount, InstanceStatus, SecretRef,
    };

    fn make_store() -> Arc<WaveStore> {
        Arc::new(WaveStore::open_in_memory().unwrap())
    }

    fn make_account(
        id: &str,
        provider: &str,
        secret_ref: SecretRef,
    ) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("{}-{}", provider, id),
            provider: provider.to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref,
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_instance(block_id: &str, identity_id: &str) -> AgentInstance {
        AgentInstance {
            id: format!("inst-{block_id}"),
            definition_id: "def-1".to_string(),
            parent_instance_id: String::new(),
            block_id: block_id.to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: identity_id.to_string(),
            memory_id: String::new(),
        }
    }

    #[test]
    fn provider_env_vars_matrix() {
        assert_eq!(provider_env_vars("github"), vec!["GITHUB_TOKEN", "GH_TOKEN"]);
        assert_eq!(provider_env_vars("anthropic"), vec!["ANTHROPIC_API_KEY"]);
        assert_eq!(provider_env_vars("openai"), vec!["OPENAI_API_KEY"]);
        assert_eq!(provider_env_vars("kimi"), vec!["MOONSHOT_API_KEY"]);
        assert_eq!(provider_env_vars("aws"), vec!["AWS_ACCESS_KEY_ID"]);
        assert!(provider_env_vars("unknown").is_empty());
    }

    #[test]
    fn resolve_plaintext_dev() {
        let s = resolve_secret(&SecretRef::PlaintextDev {
            plaintext_dev: "ghp_test123".to_string(),
        })
        .unwrap();
        assert_eq!(s, "ghp_test123");
    }

    #[test]
    fn resolve_env_var_missing() {
        let res = resolve_secret(&SecretRef::Env {
            env_var: "AGENTMUX_TEST_NEVER_SET_X9Q".to_string(),
        });
        assert!(matches!(res, Err(ResolverError::EnvVarMissing(_))));
    }

    #[test]
    fn resolve_secrets_manager_unsupported() {
        let res = resolve_secret(&SecretRef::SecretsManager {
            sm_path: "ignored".to_string(),
            sm_json_path: None,
        });
        assert!(matches!(res, Err(ResolverError::SecretsManagerUnsupported)));
    }

    #[test]
    fn inject_no_instance_does_nothing() {
        let store = make_store();
        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-no-instance", &mut env);
        assert!(env.is_empty());
    }

    #[test]
    fn inject_blank_identity_does_nothing() {
        let store = make_store();
        // Need a definition for the FK on db_agent_instances.
        let mut def = crate::backend::storage::wstore::ForgeAgent {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut def).unwrap();

        let mut inst = make_instance("block-blank", "blank");
        store.instance_create(&inst).unwrap();
        let _ = inst; // keep clippy happy

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-blank", &mut env);
        assert!(env.is_empty());
    }

    #[test]
    fn inject_full_round_trip_plaintext_dev() {
        let store = make_store();

        // Forge agent (definition).
        let mut def = crate::backend::storage::wstore::ForgeAgent {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut def).unwrap();

        // Identity bundle.
        let identity = Identity {
            id: "id-work".to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        // GitHub account (PlaintextDev for test simplicity).
        let github = make_account(
            "acct-gh",
            "github",
            SecretRef::PlaintextDev {
                plaintext_dev: "ghp_round_trip".to_string(),
            },
        );
        store.identity_upsert(&github).unwrap();
        store
            .bundle_identity_bind("id-work", "github", "acct-gh")
            .unwrap();

        // Anthropic account.
        let anthropic = make_account(
            "acct-anth",
            "anthropic",
            SecretRef::PlaintextDev {
                plaintext_dev: "sk-ant-round_trip".to_string(),
            },
        );
        store.identity_upsert(&anthropic).unwrap();
        store
            .bundle_identity_bind("id-work", "anthropic", "acct-anth")
            .unwrap();

        // Instance for the block, pointing at id-work.
        let inst = make_instance("block-1", "id-work");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-1", &mut env);

        // GitHub writes both standard env-var names from one secret.
        assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_round_trip"));
        assert_eq!(env.get("GH_TOKEN").map(String::as_str), Some("ghp_round_trip"));
        // Anthropic writes its single env var.
        assert_eq!(
            env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-ant-round_trip"),
        );
    }

    #[test]
    fn inject_partial_success_skips_failed_bindings() {
        let store = make_store();

        let mut def = crate::backend::storage::wstore::ForgeAgent {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut def).unwrap();

        let identity = Identity {
            id: "id-mixed".to_string(),
            name: "Mixed".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        // Working account.
        let good = make_account(
            "acct-good",
            "github",
            SecretRef::PlaintextDev {
                plaintext_dev: "ghp_good".to_string(),
            },
        );
        store.identity_upsert(&good).unwrap();
        store
            .bundle_identity_bind("id-mixed", "github", "acct-good")
            .unwrap();

        // Account whose Env-backed secret references a missing var.
        let bad = make_account(
            "acct-bad",
            "anthropic",
            SecretRef::Env {
                env_var: "AGENTMUX_TEST_DEFINITELY_NOT_SET_4242".to_string(),
            },
        );
        store.identity_upsert(&bad).unwrap();
        store
            .bundle_identity_bind("id-mixed", "anthropic", "acct-bad")
            .unwrap();

        let inst = make_instance("block-mixed", "id-mixed");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-mixed", &mut env);

        // GitHub injection succeeded.
        assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_good"));
        assert_eq!(env.get("GH_TOKEN").map(String::as_str), Some("ghp_good"));
        // Anthropic was skipped (env var missing) but didn't abort.
        assert!(env.get("ANTHROPIC_API_KEY").is_none());
    }

    #[test]
    fn inject_unknown_provider_is_skipped() {
        let store = make_store();

        let mut def = crate::backend::storage::wstore::ForgeAgent {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut def).unwrap();

        let identity = Identity {
            id: "id-future".to_string(),
            name: "Future".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        let custom = make_account(
            "acct-custom",
            "custom",
            SecretRef::PlaintextDev {
                plaintext_dev: "ignored".to_string(),
            },
        );
        store.identity_upsert(&custom).unwrap();
        store
            .bundle_identity_bind("id-future", "custom", "acct-custom")
            .unwrap();

        let inst = make_instance("block-future", "id-future");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-future", &mut env);
        // No env-var matrix for "custom" — nothing injected, no panic.
        assert!(env.is_empty());
    }
}
