// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Secret resolution + provider→env-var mapping.

use std::collections::HashMap;

use crate::backend::storage::wstore::{
    IdentityAccount, InstanceIdentity, SecretRef, WaveStore,
};

/// What each `IdentityAccount` resolves into. Output of `build_injection_env`.
pub type InjectionEnvVars = HashMap<String, String>;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("secret env var not set in agentmux-srv environment: {0}")]
    EnvVarMissing(String),
    #[error("identity account {0} not found")]
    AccountNotFound(String),
    #[error("secrets-manager backend not yet supported (Phase 3): {0}")]
    SecretsManagerUnsupported(String),
    #[error("wstore error: {0}")]
    Store(String),
}

/// Resolve a `SecretRef` to its plaintext value. The Phase 2 backends are
/// `Env` (read from `agentmux-srv`'s process env) and `PlaintextDev`
/// (literal value baked into the SecretRef — dev/test convenience).
pub fn resolve_secret(secret_ref: &SecretRef) -> Result<String, ResolveError> {
    match secret_ref {
        SecretRef::Env { env_var } => std::env::var(env_var)
            .map_err(|_| ResolveError::EnvVarMissing(env_var.clone())),
        SecretRef::PlaintextDev { plaintext_dev } => Ok(plaintext_dev.clone()),
        SecretRef::SecretsManager { sm_path, .. } => {
            Err(ResolveError::SecretsManagerUnsupported(sm_path.clone()))
        }
    }
}

/// Provider → list of env var names that should receive the resolved
/// secret. The mapping is a stable contract between agentmux-srv and the
/// CLIs it spawns. Issue #678 Phase 2 spec, "Credential Injection at
/// Spawn" matrix.
///
/// Most providers inject a single env var. GitHub gets two (`GITHUB_TOKEN`
/// + `GH_TOKEN`) because both are widely consumed depending on which
/// `gh` / Octokit version is in PATH.
///
/// Returns an empty slice for unknown providers — the caller logs a
/// warning but does not error (forward compatibility for new provider
/// kinds added by users).
pub fn provider_env_vars(provider: &str) -> &'static [&'static str] {
    match provider {
        "github" => &["GITHUB_TOKEN", "GH_TOKEN"],
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "kimi" => &["MOONSHOT_API_KEY"],
        // AWS multi-secret accounts (access key + secret + session) are a
        // Phase 3 concern — for now, treat the resolved value as the
        // access key id. Real AWS workflows go through the env_ref kind
        // pointing at AWS_ACCESS_KEY_ID directly.
        "aws" => &["AWS_ACCESS_KEY_ID"],
        _ => &[],
    }
}

/// Build the env-var injection map for an instance's identity bindings.
///
/// For each `(account_id, provider)` pair:
///   1. Look up the `IdentityAccount` in `db_identity_accounts`.
///   2. Resolve its `secret_ref` to a plaintext value.
///   3. Map the `provider` to its env-var slot(s) and write the value.
///
/// Failures are partial — one bad identity does not abort the whole
/// merge. The function returns the successfully-resolved map plus a
/// list of `(account_id, error)` pairs the caller can log. This matches
/// the issue spec's "Missing identity at launch: warn, don't block —
/// fall back to ambient host credentials" decision.
pub fn build_injection_env(
    wstore: &WaveStore,
    identities: &[InstanceIdentity],
) -> (InjectionEnvVars, Vec<(String, ResolveError)>) {
    let mut env: InjectionEnvVars = HashMap::new();
    let mut errors: Vec<(String, ResolveError)> = Vec::new();

    for ident in identities {
        let account = match wstore.identity_get(&ident.account_id) {
            Ok(Some(a)) => a,
            Ok(None) => {
                errors.push((
                    ident.account_id.clone(),
                    ResolveError::AccountNotFound(ident.account_id.clone()),
                ));
                continue;
            }
            Err(e) => {
                errors.push((ident.account_id.clone(), ResolveError::Store(e.to_string())));
                continue;
            }
        };

        let value = match resolve_secret(&account.secret_ref) {
            Ok(v) => v,
            Err(e) => {
                errors.push((ident.account_id.clone(), e));
                continue;
            }
        };

        let target_vars = provider_env_vars(&ident.provider);
        if target_vars.is_empty() {
            // Unknown provider — log via the error channel but do not
            // fail. Account.context may carry a hint but we don't fall
            // back to it in Phase 2.
            tracing::warn!(
                account_id = %ident.account_id,
                provider = %ident.provider,
                "no env-var mapping for provider; identity has no effect this turn"
            );
            // Keep the loop going so other identities still inject.
            let _ = account; // silence unused warning when no vars target it
            continue;
        }
        for var in target_vars {
            env.insert((*var).to_string(), value.clone());
        }
    }

    (env, errors)
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::wstore::IdentityAccount;

    fn dev_account(provider: &str, value: &str) -> IdentityAccount {
        IdentityAccount {
            id: format!("acc-{provider}"),
            name: format!("dev-{provider}"),
            provider: provider.to_string(),
            kind: "api_key".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::PlaintextDev {
                plaintext_dev: value.to_string(),
            },
            context: serde_json::Value::Object(serde_json::Map::new()),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn resolve_plaintext_dev_returns_literal() {
        let s = SecretRef::PlaintextDev {
            plaintext_dev: "abc123".to_string(),
        };
        assert_eq!(resolve_secret(&s).unwrap(), "abc123");
    }

    #[test]
    fn resolve_env_var_missing_errors() {
        let s = SecretRef::Env {
            env_var: "AGENTMUX_TEST_DOES_NOT_EXIST_ZZZ".to_string(),
        };
        let r = resolve_secret(&s);
        assert!(matches!(r, Err(ResolveError::EnvVarMissing(_))));
    }

    #[test]
    fn resolve_env_var_present_returns_value() {
        // SAFETY: setting a process-scoped env var inside the test binary.
        // Keep the var name unlikely-to-collide.
        unsafe {
            std::env::set_var("AGENTMUX_TEST_RESOLVE_OK", "yep");
        }
        let s = SecretRef::Env {
            env_var: "AGENTMUX_TEST_RESOLVE_OK".to_string(),
        };
        assert_eq!(resolve_secret(&s).unwrap(), "yep");
        unsafe {
            std::env::remove_var("AGENTMUX_TEST_RESOLVE_OK");
        }
    }

    #[test]
    fn resolve_secrets_manager_returns_unsupported() {
        let s = SecretRef::SecretsManager {
            sm_path: "/foo/bar".to_string(),
            sm_json_path: None,
        };
        let r = resolve_secret(&s);
        assert!(matches!(r, Err(ResolveError::SecretsManagerUnsupported(_))));
    }

    #[test]
    fn provider_env_vars_known_providers() {
        assert_eq!(provider_env_vars("github"), &["GITHUB_TOKEN", "GH_TOKEN"]);
        assert_eq!(provider_env_vars("anthropic"), &["ANTHROPIC_API_KEY"]);
        assert_eq!(provider_env_vars("openai"), &["OPENAI_API_KEY"]);
        assert_eq!(provider_env_vars("kimi"), &["MOONSHOT_API_KEY"]);
        assert!(provider_env_vars("unknown-provider").is_empty());
    }

    #[test]
    fn build_injection_env_resolves_known_provider() {
        let store = WaveStore::open_in_memory().unwrap();
        let acc = dev_account("github", "ghp_test");
        store.identity_upsert(&acc).unwrap();

        let (env, errs) = build_injection_env(
            &store,
            &[InstanceIdentity {
                account_id: acc.id.clone(),
                provider: "github".to_string(),
            }],
        );

        assert!(errs.is_empty());
        assert_eq!(env.get("GITHUB_TOKEN"), Some(&"ghp_test".to_string()));
        assert_eq!(env.get("GH_TOKEN"), Some(&"ghp_test".to_string()));
    }

    #[test]
    fn build_injection_env_records_unknown_account() {
        let store = WaveStore::open_in_memory().unwrap();
        let (env, errs) = build_injection_env(
            &store,
            &[InstanceIdentity {
                account_id: "ghost".to_string(),
                provider: "github".to_string(),
            }],
        );
        assert!(env.is_empty());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].0, "ghost");
        assert!(matches!(errs[0].1, ResolveError::AccountNotFound(_)));
    }

    #[test]
    fn build_injection_env_partial_success_other_identities_still_inject() {
        let store = WaveStore::open_in_memory().unwrap();
        let acc = dev_account("github", "ghp_ok");
        store.identity_upsert(&acc).unwrap();

        let (env, errs) = build_injection_env(
            &store,
            &[
                InstanceIdentity {
                    account_id: "ghost".to_string(),
                    provider: "anthropic".to_string(),
                },
                InstanceIdentity {
                    account_id: acc.id.clone(),
                    provider: "github".to_string(),
                },
            ],
        );

        assert_eq!(env.get("GITHUB_TOKEN"), Some(&"ghp_ok".to_string()));
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn build_injection_env_unknown_provider_skipped_silently() {
        let store = WaveStore::open_in_memory().unwrap();
        let acc = dev_account("custom-foo", "x");
        store.identity_upsert(&acc).unwrap();

        let (env, errs) = build_injection_env(
            &store,
            &[InstanceIdentity {
                account_id: acc.id.clone(),
                provider: "custom-foo".to_string(),
            }],
        );
        assert!(env.is_empty());
        // Unknown provider = warn-only, no error returned to caller.
        assert!(errs.is_empty());
    }
}
