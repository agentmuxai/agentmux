// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! [`resolve_secret`]: turning a `SecretRef` into a plaintext credential.
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change).

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::backend::storage::store::{SecretRef, Store};
use crate::backend::vault::aws_import::{self, Source};
use crate::identity::aws_broker::mint::{HostRunner, Runner};

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
/// - **SecretsManager**: read from AWS Secrets Manager with the host's own AWS login, the whole secret or the
///   field `sm_json_path` names. A failed read is [`ResolverError::SecretsManagerRead`].
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
    if matches!(account.secret_ref, SecretRef::Vault { .. }) {
        return crate::backend::vault::account_key::read(identity_store, &account.id, &account.secret_ref)
            .map(|key| crate::identity::oauth_tokens::bearer_value(key.to_string())).map_err(ResolverError::VaultError);
    }
    resolve_stored(&account.secret_ref).map(crate::identity::oauth_tokens::bearer_value)
}

/// An AWS Secrets Manager secret's text (or the field `json_path` names in a JSON one), read with the host's own AWS
/// login by the reader the Vault's backed entries use (`vault::aws_import::read_value_blocking`). On the blocking thread
/// the injection already runs on.
///
/// The model gateway asks for an account's key on every request, and each ask here is a run of the AWS CLI and a
/// billed Secrets Manager call, so a value read is kept for [`SM_CACHE_TTL`]: a rotation reaches the next request
/// within that time. A failed read is not kept. (A Vault backed entry reads at each use and keeps nothing.)
fn secrets_manager_value<R: Runner>(runner: &R, path: &str, json_path: Option<&str>) -> Result<String, ResolverError> {
    secrets_manager_value_at(runner, path, json_path, Instant::now())
}

/// How long a value read from Secrets Manager is reused by the resolver.
const SM_CACHE_TTL: Duration = Duration::from_secs(30);

type SmCache = BTreeMap<(String, Option<String>), (Instant, Zeroizing<String>)>;
static SM_CACHE: Mutex<SmCache> = Mutex::new(BTreeMap::new());

fn secrets_manager_value_at<R: Runner>(runner: &R, path: &str, json_path: Option<&str>, now: Instant) -> Result<String, ResolverError> {
    let place = (path.to_string(), json_path.map(str::to_string));
    if let Some((read_at, value)) = SM_CACHE.lock().unwrap_or_else(PoisonError::into_inner).get(&place) {
        if now.saturating_duration_since(*read_at) < SM_CACHE_TTL {
            return Ok(value.to_string());
        }
    }
    let source = Source { aws_secret_id: path.to_string(), key: json_path.map(str::to_string), ..Default::default() };
    let source = aws_import::check_source(&source).map_err(ResolverError::SecretsManagerRead)?;
    let value = aws_import::read_value_blocking(runner, &source).map_err(ResolverError::SecretsManagerRead)?;
    let mut cache = SM_CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|_, (read_at, _)| now.saturating_duration_since(*read_at) < SM_CACHE_TTL);
    cache.insert(place, (now, value.clone()));
    Ok(value.to_string())
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
        SecretRef::SecretsManager { sm_path, sm_json_path } => secrets_manager_value(&HostRunner, sm_path, sm_json_path.as_deref()),
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
        SecretRef::Vault { .. } => Err(ResolverError::VaultError("an account is required to read a Vault key".into())),
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

    /// A secret in AWS Secrets Manager resolves through the reader the Vault's backed entries use (the AWS CLI is a
    /// stand-in, the secret invented): the whole text, or the field `sm_json_path` names.
    #[test]
    fn resolve_secrets_manager_reads_the_secret_or_a_field_of_it() {
        use crate::backend::vault::backed::tests::Aws;
        let aws = Aws::with(&[("svc/example-key", Ok(r#"{"db": {"password": "pw-from-aws"}}"#)), ("svc/plain", Ok("plain-from-aws"))]);
        assert_eq!(secrets_manager_value(&aws, "svc/example-key", Some("db.password")).unwrap(), "pw-from-aws");
        assert_eq!(secrets_manager_value(&aws, "svc/plain", None).unwrap(), "plain-from-aws");
        let missing = secrets_manager_value(&aws, "svc/absent", None);
        assert!(matches!(&missing, Err(ResolverError::SecretsManagerRead(why)) if why.contains("AWS has no secret")), "{missing:?}");
        // The id goes to the CLI as an argument: one that isn't a secret's name never gets there.
        assert!(matches!(secrets_manager_value(&aws, "--query x", None), Err(ResolverError::SecretsManagerRead(_))));
    }

    /// The model gateway asks for a key on every request: a value read is reused for 30 s, then read again, and a
    /// failed read is not kept.
    #[test]
    fn resolve_secrets_manager_reuses_a_read_for_a_short_while() {
        use crate::backend::vault::backed::tests::Aws;
        let aws = Aws::with(&[("svc/rotating-key", Ok("first-from-aws"))]);
        let t0 = Instant::now();
        assert_eq!(secrets_manager_value_at(&aws, "svc/rotating-key", None, t0).unwrap(), "first-from-aws");
        aws.change("svc/rotating-key", "second-from-aws");
        assert_eq!(secrets_manager_value_at(&aws, "svc/rotating-key", None, t0 + Duration::from_secs(10)).unwrap(), "first-from-aws");
        assert_eq!(aws.reads(), 1, "a second ask within the time costs no call");
        assert_eq!(secrets_manager_value_at(&aws, "svc/rotating-key", None, t0 + Duration::from_secs(31)).unwrap(), "second-from-aws");
        assert_eq!(aws.reads(), 2);

        let down = Aws::with(&[]);
        assert!(secrets_manager_value_at(&down, "svc/not-yet-there", None, t0).is_err());
        assert_eq!(down.reads(), 1);
        assert!(secrets_manager_value_at(&down, "svc/not-yet-there", None, t0).is_err());
        assert_eq!(down.reads(), 2, "a failed read is asked again, not kept");
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
