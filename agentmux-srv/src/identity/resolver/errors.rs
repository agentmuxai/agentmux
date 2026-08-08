// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolver error types: [`SpawnGateError`] and [`ResolverError`].
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change).

use crate::backend::storage::error::StoreError;

/// Blocking spawn-gate error — layer 3 of
/// SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md (§2.2).
///
/// Returned by the injection entry points when an **oauth-class** provider
/// the agent is supposed to have credentials for (a binding exists, or the
/// provider is the agent definition's own CLI provider) has no resolvable
/// account AND the agent has not opted into ambient login
/// (`use_ambient_login = 0`, the default). The spawn callers surface
/// `Display` verbatim in the agent pane (same `error_during_execution`
/// frame other spawn failures use) — the wording is the spec's.
///
/// Api-key-class bindings keep the historical log-and-skip behavior; this
/// error exists only for the oauth class, where silent fallback meant the
/// CLI would read the user's global login (`~/.claude`) after an account
/// was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnGateError {
    /// The gate's credentials verdict: no resolvable account, no opt-in.
    MissingCredentials { provider: String },
    /// The injection task itself could not run to completion (task-join
    /// failure — e.g. a panic inside the blocking closure, which also
    /// poisons the `Store` mutex for every later call). The gate FAILS
    /// CLOSED on this: an open fallback would silently convert one panic
    /// anywhere in the store into a permanent, systemic bypass of
    /// `use_ambient_login = false` (reagent P1, PR #2164 round 1). A
    /// blocked spawn is retryable and visible; a silent ambient launch
    /// is neither.
    InjectionUnavailable { detail: String },
}

impl std::fmt::Display for SpawnGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // "Bind an account in the Armory" is now the ONLY path — the
            // ambient/"use global CLI login" opt-in this used to also
            // suggest was retired (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_
            // 2026_07_20.md §7): it let a credential the app couldn't
            // attribute to any account run indefinitely, invisible to
            // Armory. Don't point users at a toggle that no longer works.
            SpawnGateError::MissingCredentials { provider } => write!(
                f,
                "no credentials for {}: the bound account was deleted or is \
                 unresolvable. Bind an account for this provider in the Armory.",
                provider,
            ),
            SpawnGateError::InjectionUnavailable { detail } => write!(
                f,
                "credential injection could not run ({detail}); the spawn was \
                 refused rather than falling back to the global CLI login. \
                 Retry, and check `muxlog auth` if it persists.",
            ),
        }
    }
}

impl std::error::Error for SpawnGateError {}

/// Errors specific to the resolver. Every variant is recoverable
/// (the spawn proceeds with whatever env vars resolved successfully)
/// — they exist for tracing visibility, not control flow.
#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
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

    /// The OS keychain read failed (no entry, locked store, or no Secret
    /// Service agent). Armory API keys (`SecretRef::Keychain`) live
    /// in the OS keychain; this surfaces a resolution failure at spawn.
    #[error("keychain error: {0}")]
    KeychainError(String),

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}
