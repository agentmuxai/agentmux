// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! [`resolve_secret`]: turning a `SecretRef` into a plaintext credential.
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change).

use crate::backend::storage::store::SecretRef;

use super::errors::ResolverError;

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
        SecretRef::Keychain { account, .. } => {
            // Armory API keys: pull the plaintext from the OS
            // keychain at spawn time. The account string is
            // `acct:<account_id>`; secret_store reconstructs the key
            // from the id, so strip the namespace prefix here.
            let account_id = account.strip_prefix("acct:").unwrap_or(account);
            crate::identity::secret_store::get(account_id)
                .map(|z| z.to_string())
                .map_err(ResolverError::KeychainError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
