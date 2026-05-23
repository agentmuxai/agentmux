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

    /// `OAuthConfigDir` is a filesystem pointer, not a secret string —
    /// `resolve_secret` cannot turn it into a credential value because
    /// the credential lives in a CLI-managed token file inside the dir.
    /// Oauth-class providers must be routed through the config-dir
    /// injection path that PR B adds to `inject_identity_env`. Seeing
    /// this error from `resolve_secret` means the caller forgot to
    /// dispatch by provider class first.
    #[error("OAuthConfigDir is a config-dir pointer, not a resolvable secret — routed via the oauth-class injection path, not resolve_secret")]
    OAuthConfigDirNotASecret,

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

/// What kind of credential a provider uses, and how
/// `inject_identity_env` puts it into the agent's env at spawn time.
/// Per `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderClass {
    /// **API-key class.** The binding's `SecretRef` resolves to a
    /// single secret string, injected as the listed env vars. All
    /// listed vars receive the same value — multi-var emission
    /// covers "two CLIs want different var names for the same secret"
    /// (e.g. github writes both `GITHUB_TOKEN` and `GH_TOKEN`).
    ApiKey { env_vars: &'static [&'static str] },
    /// **OAuth class.** The binding's `SecretRef` is a
    /// `SecretRef::OAuthConfigDir` pointer; the resolver sets
    /// `config_dir_env_var = <dir>` at spawn so the CLI reads its
    /// OAuth tokens from the per-bundle directory.
    OAuth { config_dir_env_var: &'static str },
}

/// Classify a provider id. `None` for unknown providers — the
/// resolver logs and skips them.
pub fn provider_class(provider: &str) -> Option<ProviderClass> {
    match provider {
        // ── API-key class ─────────────────────────────────────────
        // ApiKey.env_vars values match the legacy provider_env_vars
        // matrix exactly — the new dispatch is additive.
        "github" => Some(ProviderClass::ApiKey {
            env_vars: &["GITHUB_TOKEN", "GH_TOKEN"],
        }),
        "anthropic" => Some(ProviderClass::ApiKey {
            env_vars: &["ANTHROPIC_API_KEY"],
        }),
        "openai" => Some(ProviderClass::ApiKey {
            env_vars: &["OPENAI_API_KEY"],
        }),
        "kimi" => Some(ProviderClass::ApiKey {
            env_vars: &["MOONSHOT_API_KEY"],
        }),
        "aws" => Some(ProviderClass::ApiKey {
            env_vars: &["AWS_ACCESS_KEY_ID"],
        }),
        // ── OAuth class ───────────────────────────────────────────
        // Env-var names come from the CLI provider registry
        // (`agentmux-srv/src/backend/providers.rs` —
        // `ProviderConfig::auth_config_dir_env_var`) so the resolver
        // can never drift from the launcher spawn path: there is one
        // source of truth per CLI for which env var redirects its
        // config / auth directory. The match arm enumerates which
        // providers we currently treat as OAuth-class for identity
        // bundles (claude / codex / openclaw — per spec §4.3); the
        // env-var string is read from the registry, not duplicated.
        "claude" | "codex" | "openclaw" => {
            crate::backend::providers::get_provider(provider).map(|cfg| {
                ProviderClass::OAuth {
                    config_dir_env_var: cfg.auth_config_dir_env_var,
                }
            })
        }
        _ => None,
    }
}

/// Legacy convenience: env vars for an api-key provider. Delegates to
/// [`provider_class`]; returns empty for oauth-class providers (their
/// resolution path doesn't go through string-secret env-var injection)
/// and for unknown providers.
pub fn provider_env_vars(provider: &str) -> Vec<&'static str> {
    match provider_class(provider) {
        Some(ProviderClass::ApiKey { env_vars }) => env_vars.to_vec(),
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
        SecretRef::OAuthConfigDir { dir } => {
            // Read but ignored — the resolver doesn't consume the
            // pointer; PR B's oauth-class dispatch in
            // `inject_identity_env` reads `dir` and sets the
            // provider's config-dir env var directly, bypassing
            // `resolve_secret` entirely.
            let _ = dir;
            Err(ResolverError::OAuthConfigDirNotASecret)
        }
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
        // Empty or legacy "blank" sentinel → ambient creds (no
        // injection). The UI no longer produces these for new
        // launches (identity is now required at submit-time —
        // SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md), so seeing
        // one here means either a legacy continuation row or a UI
        // regression. Warn so the regression is visible in logs.
        tracing::warn!(
            target: "identity",
            "instance {} has empty/blank identity_id — falling back to ambient creds. \
             Legacy row or UI regression?",
            block_id
        );
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

    // Step 4: per-binding resolution + env injection.
    //
    // Each binding's provider determines HOW its account contributes
    // to the agent's env (SPEC_OAUTH_IDENTITY_BUNDLES §4.3):
    //   - ApiKey  — resolve secret_ref to a string, inject as env var(s).
    //   - OAuth   — expect SecretRef::OAuthConfigDir, inject its dir
    //               as the provider's config-dir env var.
    //
    // Per-binding failures (unknown provider, account row missing,
    // mismatched secret_ref, secret resolution failed) are logged and
    // skipped — other bindings still inject.
    for binding in &bindings {
        let class = match provider_class(&binding.provider) {
            Some(c) => c,
            None => {
                tracing::warn!(
                    target: "identity",
                    "no provider class for {} (binding for identity {}) — skipping",
                    binding.provider,
                    instance.identity_id,
                );
                continue;
            }
        };

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

        match class {
            ProviderClass::ApiKey { env_vars: env_keys } => {
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
                let env_key_count = env_keys.len();
                for key in env_keys {
                    env_vars.insert(key.to_string(), secret.clone());
                }
                tracing::info!(
                    target: "identity",
                    "injected {} env var(s) for api-key provider {} (identity={}, account={})",
                    env_key_count,
                    binding.provider,
                    instance.identity_id,
                    binding.account_id,
                );
            }
            ProviderClass::OAuth { config_dir_env_var } => {
                // OAuth-class bindings expect SecretRef::OAuthConfigDir.
                // Any other variant is a misconfiguration — log and
                // skip rather than mis-inject the wrong secret.
                let dir = match &account.secret_ref {
                    SecretRef::OAuthConfigDir { dir } => dir.clone(),
                    other => {
                        tracing::warn!(
                            target: "identity",
                            "oauth-class provider {} has non-OAuthConfigDir secret_ref \
                             ({:?}) on account {} — skipping",
                            binding.provider,
                            other,
                            binding.account_id,
                        );
                        continue;
                    }
                };
                env_vars.insert(config_dir_env_var.to_string(), dir);
                tracing::info!(
                    target: "identity",
                    "injected {} for oauth provider {} (identity={}, account={})",
                    config_dir_env_var,
                    binding.provider,
                    instance.identity_id,
                    binding.account_id,
                );
            }
        }
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
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
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

    // PlaintextDev-using tests are gated behind cfg(debug_assertions)
    // because release builds reject PlaintextDev with
    // ResolverError::PlaintextDevDisabledInRelease, so the assertions
    // below would fail under `cargo test --release`. Reagent P2
    // (PR #751). The Env / SecretsManager / unknown-provider paths
    // are tested separately and have no debug-only dependency.

    #[cfg(debug_assertions)]
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
    fn provider_class_oauth_providers() {
        // Spec §4.3 — the three known oauth providers must classify
        // as OAuth with the SAME config-dir env vars the CLI provider
        // registry defines (single source of truth). Pinning the
        // expected strings here catches drift in either direction —
        // if the registry changes a value, this test fails and the
        // change becomes deliberate.
        assert_eq!(
            provider_class("claude"),
            Some(ProviderClass::OAuth { config_dir_env_var: "CLAUDE_CONFIG_DIR" }),
        );
        assert_eq!(
            provider_class("codex"),
            Some(ProviderClass::OAuth { config_dir_env_var: "CODEX_HOME" }),
        );
        assert_eq!(
            provider_class("openclaw"),
            Some(ProviderClass::OAuth { config_dir_env_var: "OPENCLAW_HOME" }),
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_sets_config_dir_env_var() {
        let store = make_store();

        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let identity = Identity {
            id: "id-oauth".to_string(),
            name: "OAuth".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        let claude = make_account(
            "acct-claude",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-oauth/claude".to_string(),
            },
        );
        store.identity_upsert(&claude).unwrap();
        store
            .bundle_identity_bind("id-oauth", "claude", "acct-claude")
            .unwrap();

        let inst = make_instance("block-oauth", "id-oauth");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-oauth", &mut env);

        // OAuth dispatch sets the provider's config-dir env var.
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-oauth/claude"),
        );
        // And does NOT set the anthropic api-key env var — dispatch
        // is by provider class, not by token shape.
        assert!(env.get("ANTHROPIC_API_KEY").is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_skips_account_with_non_oauth_secret_ref() {
        // An oauth-class provider (claude) bound to an account whose
        // SecretRef is the API-key shape (Env) is a misconfiguration:
        // the resolver logs + skips rather than mis-injecting the
        // wrong secret as if it were a config-dir.
        let store = make_store();

        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let identity = Identity {
            id: "id-bad".to_string(),
            name: "Bad".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        let bad = make_account(
            "acct-bad",
            "claude",
            SecretRef::Env {
                env_var: "CLAUDE_TOKEN_NOT_A_DIR".to_string(),
            },
        );
        store.identity_upsert(&bad).unwrap();
        store
            .bundle_identity_bind("id-bad", "claude", "acct-bad")
            .unwrap();

        let inst = make_instance("block-bad", "id-bad");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-bad", &mut env);

        // Nothing injected — the binding was skipped.
        assert!(env.get("CLAUDE_CONFIG_DIR").is_none());
        assert!(env.is_empty());
    }

    #[test]
    fn resolve_oauth_config_dir_is_not_a_secret() {
        // OAuthConfigDir is a pointer to a CLI-managed token directory,
        // not a resolvable secret string. PR B's oauth-class dispatch
        // in `inject_identity_env` reads `dir` and sets the provider's
        // config-dir env var directly, bypassing `resolve_secret`. The
        // error here is a guard against a caller forgetting that
        // dispatch — pre-PR-B nothing produces this variant, but the
        // arm has to exist for the match to be exhaustive.
        let res = resolve_secret(&SecretRef::OAuthConfigDir {
            dir: "/path/to/bundle/claude".to_string(),
        });
        assert!(matches!(res, Err(ResolverError::OAuthConfigDirNotASecret)));
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
        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let mut inst = make_instance("block-blank", "blank");
        store.instance_create(&inst).unwrap();
        let _ = inst; // keep clippy happy

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store, "block-blank", &mut env);
        assert!(env.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_full_round_trip_plaintext_dev() {
        let store = make_store();

        // Agent definition.
        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

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

    #[cfg(debug_assertions)]
    #[test]
    fn inject_partial_success_skips_failed_bindings() {
        let store = make_store();

        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

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

    #[cfg(debug_assertions)]
    #[test]
    fn inject_unknown_provider_is_skipped() {
        let store = make_store();

        let mut def = crate::backend::storage::wstore::AgentDefinition {
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
            updated_at: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

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
