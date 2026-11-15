// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! [`resolve_secret`]: turning a `SecretRef` into a plaintext credential.
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change).

use crate::backend::storage::store::{SecretRef, Store};

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
///
/// The value is the credential to use: an account connected with OAuth
/// stores a JSON token blob, of which only its `access_token` is the
/// credential (`oauth_tokens::bearer_value`). A plain key is unchanged.
pub fn resolve_secret(secret_ref: &SecretRef) -> Result<String, ResolverError> {
    resolve_stored(secret_ref).map(crate::identity::oauth_tokens::bearer_value)
}

/// An account connected with OAuth (GitHub's tokens last hours) is refreshed before its token goes
/// into an agent's environment, so a launch after the expiry doesn't hand out a dead token. Best
/// effort: if it can't be refreshed, the stored token is used as it was, and a grant the provider
/// no longer accepts marks the account as needing to be connected again (`oauth_tokens`). Runs on
/// the blocking thread the injection already uses; with no runtime (a test), nothing happens.
fn refresh_oauth_token_if_due(account: &crate::backend::storage::store::IdentityAccount, id_store: &std::sync::Arc<Store>, identity_store: &std::sync::Arc<Store>) {
    if account.kind != "oauth" || !matches!(account.secret_ref, SecretRef::Keychain { .. }) {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let result = runtime.block_on(crate::identity::oauth_tokens::access_token_for(
        &crate::identity::oauth_tokens::LiveHttp,
        &crate::identity::oauth_tokens::KeychainSecrets,
        id_store,
        identity_store,
        account,
        Some(id_store),
    ));
    if let Err(e) = result {
        tracing::info!(target: "identity", account_id = %account.id, error = %e, "identity.oauth: the token couldn't be refreshed before launch; using the stored one");
    }
}

/// [`resolve_secret`] for an account about to go into an agent's environment: an OAuth token that is
/// due is refreshed first.
pub fn resolve_account_secret(
    account: &crate::backend::storage::store::IdentityAccount,
    id_store: &std::sync::Arc<Store>,
    identity_store: &std::sync::Arc<Store>,
) -> Result<String, ResolverError> {
    refresh_oauth_token_if_due(account, id_store, identity_store);
    resolve_secret(&account.secret_ref)
}

fn resolve_stored(secret_ref: &SecretRef) -> Result<String, ResolverError> {
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
            // Account API keys: pull the plaintext from the OS
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

    /// An OAuth-connected GitHub account stores a JSON blob; the agent's
    /// `GH_TOKEN` is its access token, not the blob (spec §2.3's bug).
    #[cfg(debug_assertions)]
    #[test]
    fn resolve_an_oauth_token_blob_to_its_access_token() {
        let s = resolve_secret(&SecretRef::PlaintextDev {
            plaintext_dev: r#"{"access_token":"gho_oauth","refresh_token":null,"expires_at":0}"#.to_string(),
        })
        .unwrap();
        assert_eq!(s, "gho_oauth");
    }

    #[test]
    fn resolve_env_var_blob_and_plain_values() {
        let var = "AGENTMUX_TEST_RESOLVE_SECRET_BLOB_Q7Z";
        std::env::set_var(var, r#"{"access_token":"gho_env"}"#);
        assert_eq!(resolve_secret(&SecretRef::Env { env_var: var.to_string() }).unwrap(), "gho_env");
        std::env::set_var(var, "ghp_plain_env");
        assert_eq!(resolve_secret(&SecretRef::Env { env_var: var.to_string() }).unwrap(), "ghp_plain_env");
        std::env::remove_var(var);
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
