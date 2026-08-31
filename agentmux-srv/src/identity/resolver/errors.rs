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
    /// The bound account's `SecretRef::OAuthConfigDir` resolves to the
    /// provider's own literal ambient home directory (e.g. `~/.claude`)
    /// instead of an AgentMux-isolated dir. Blocked unconditionally,
    /// regardless of `use_ambient_login` — a real, currently-live account
    /// configured exactly this way was found in this repo's own data
    /// (`docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md`
    /// §8); this variant is the enforcement that closes that gap. See
    /// `docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`.
    AmbientHomeDirNotAllowed { provider: String, dir: String },
    /// Failed to seed the isolated Claude Code config dir's `CLAUDE.md`
    /// placeholder (`providers::seed_claude_md_placeholder_if_missing`).
    /// Blocked rather than warn-and-continue: an isolated dir with no
    /// `CLAUDE.md` of its own is exactly the condition where Claude Code
    /// CLI falls through to the operator's real `~/.claude/CLAUDE.md` —
    /// continuing to spawn would launch the agent with the leak this
    /// fix exists to close. See
    /// `docs/specs/SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md`.
    ClaudeMdSeedFailed { provider: String, dir: String, error: String },
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
            SpawnGateError::AmbientHomeDirNotAllowed { provider, dir } => write!(
                f,
                "this agent's {provider} identity points directly at your personal \
                 {provider} config directory ({dir}) instead of an isolated AgentMux \
                 account — AgentMux no longer allows spawning an agent against your \
                 own global CLI login. Re-bind this identity to an isolated account \
                 in Armory → Accounts (delete the current {provider} account and log \
                 in again to create a fresh, isolated one), then retry.",
            ),
            SpawnGateError::ClaudeMdSeedFailed { provider, dir, error } => write!(
                f,
                "could not isolate this agent's {provider} config directory ({dir}): \
                 {error}. Refusing to spawn with an unprotected config dir — retry, \
                 and check the directory's permissions if it persists.",
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
