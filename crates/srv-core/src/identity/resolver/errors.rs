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
/// account. There is no per-agent opt-out. The spawn callers surface
/// `Display` verbatim in the agent pane (same `error_during_execution`
/// frame other spawn failures use) — the wording is the spec's.
///
/// Api-key-class bindings keep the historical log-and-skip behavior; this
/// error exists only for the oauth class, where silent fallback meant the
/// CLI would read the user's global login (`~/.claude`) after an account
/// was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnGateError {
    /// The gate's credentials verdict: no resolvable account. `linked` says
    /// whether the agent has a link for the provider at all: `false` is an
    /// agent no account was ever linked to (or whose link went with its
    /// account), `true` a link to an account that is gone or unusable.
    MissingCredentials { provider: String, linked: bool },
    /// The injection task itself could not run to completion (task-join
    /// failure — e.g. a panic inside the blocking closure, which also
    /// poisons the `Store` mutex for every later call). The gate FAILS
    /// CLOSED on this: an open fallback would silently convert one panic
    /// anywhere in the store into a permanent, systemic bypass of the
    /// oauth gate (#2164). A
    /// blocked spawn is retryable and visible; a silent ambient launch
    /// is neither.
    InjectionUnavailable { detail: String },
    /// The bound account's `SecretRef::OAuthConfigDir` resolves to the
    /// provider's own literal ambient home directory (e.g. `~/.claude`)
    /// instead of an AgentMux-isolated dir. Blocked unconditionally — a
    /// real, currently-live account
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
    /// The block names an agent that was deleted (its row is gone and the
    /// shared definition registry holds only a retired record for it). The
    /// pane survived the delete — deleted from another window, or mid-spawn
    /// — and used to spawn on the shared login with no identity at all; it
    /// is refused instead (#3577).
    AgentDeleted { agent_id: String },
}

impl std::fmt::Display for SpawnGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Starts "this agent's" so muxspect's `classify_last_error_source`
            // files it under "identity" with the other gate refusals, and
            // carries "was deleted, so this pane cannot start" — the phrase
            // `agents/failure.rs` classifies it by (no Retry: a retry is
            // refused identically).
            SpawnGateError::AgentDeleted { agent_id } => write!(
                f,
                "this agent's definition ({agent_id}) was deleted, so this pane cannot \
                 start. Close the pane, or launch another agent in it.",
            ),
            // "Bind an account in Connectors" is now the ONLY path — the
            // ambient/"use global CLI login" opt-in this used to also
            // suggest was retired (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_
            // 2026_07_20.md §7): it let a credential the app couldn't
            // attribute to any account run indefinitely, invisible to
            // Connectors. Don't point users at a toggle that no longer works.
            // Both keep "no credentials for {provider}:" and "Bind an account
            // for this provider in Connectors", which `agents/failure.rs` and
            // muxspect classify the refusal by.
            SpawnGateError::MissingCredentials { provider, linked: true } => write!(
                f,
                "no credentials for {}: the bound account was deleted or is \
                 unresolvable. Bind an account for this provider in Connectors → Accounts.",
                provider,
            ),
            SpawnGateError::MissingCredentials { provider, linked: false } => write!(
                f,
                "no credentials for {provider}: no {provider} account is linked to this \
                 agent. Bind an account for this provider in Connectors → Accounts.",
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
                 in Connectors → Accounts (delete the current {provider} account and log \
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

/// The `linked` of a no-binding refusal for `provider`, following the
/// binding lookup: the agent's own links, or its template's (`""` for none)
/// only when the agent has none. `false` only when those links were read and
/// none is for the provider. The binding lookup turns a failed read into an
/// empty list, so a read that fails here too counts as linked: the refusal
/// then says the account is unresolvable, not that none is linked.
pub(super) fn refusal_linked(identity_store: &crate::backend::storage::store::Store, agent_id: &str, template_id: &str, provider: &str) -> bool {
    let for_provider = |links: Vec<crate::backend::storage::identities::AgentIdentityLink>| {
        links.iter().any(|l| crate::backend::providers::resolve_provider_alias(&l.provider) == provider)
    };
    match identity_store.agent_identity_list_for_agent(agent_id) {
        Err(_) => true,
        Ok(own) if !own.is_empty() => for_provider(own),
        Ok(_) if template_id.is_empty() => false,
        Ok(_) => identity_store.agent_identity_list_for_agent(template_id).map_or(true, for_provider),
    }
}

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
    #[cfg(not(debug_assertions))]
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
    /// Service agent). Account API keys (`SecretRef::Keychain`) live
    /// in the OS keychain; this surfaces a resolution failure at spawn.
    #[error("keychain error: {0}")]
    KeychainError(String),

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use super::refusal_linked;
    use crate::backend::storage::agents::test_agent_def;
    use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};

    fn agent(store: &Store, id: &str) {
        store.agent_def_insert(&mut test_agent_def(id, id, "claude", "host", 0, "")).unwrap();
    }

    fn link(store: &Store, agent_id: &str, provider: &str) {
        let account = format!("acct-{agent_id}-{provider}");
        store
            .identity_upsert(&IdentityAccount {
                id: account.clone(),
                name: account.clone(),
                provider: provider.to_string(),
                kind: "pat".to_string(),
                display_name: String::new(),
                secret_ref: SecretRef::OAuthConfigDir { dir: String::new() },
                context: serde_json::json!({}),
                status: "unknown".to_string(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.agent_identity_link(agent_id, &account, provider).unwrap();
    }

    /// "Not linked" only when the links were read and none is for the provider:
    /// the agent's own, or its template's when it has a template.
    #[test]
    fn a_refusal_is_unlinked_only_when_no_link_for_the_provider_was_found() {
        let store = Store::open_in_memory().unwrap();
        for id in ["def-none", "def-other", "def-own", "tmpl"] {
            agent(&store, id);
        }
        assert!(!refusal_linked(&store, "def-none", "", "claude"), "no links at all");
        link(&store, "def-other", "codex");
        assert!(!refusal_linked(&store, "def-other", "", "claude"), "a link for another provider only");
        link(&store, "def-own", "claude");
        assert!(refusal_linked(&store, "def-own", "", "claude"), "a link the gate couldn't resolve");
        link(&store, "tmpl", "claude");
        assert!(refusal_linked(&store, "def-none", "tmpl", "claude"), "its template's link");
        assert!(!refusal_linked(&store, "def-other", "tmpl", "claude"), "an agent with links of its own doesn't use its template's");
    }
}
