// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity → env-var resolver.
//!
//! Per-provider matrix of which env vars carry which credential. The
//! GitHub PAT becomes both `GITHUB_TOKEN` and `GH_TOKEN` because both
//! the official `gh` CLI and direct API consumers (curl, oct.js) read
//! one or the other; emitting both is the lowest-friction way to make
//! every common workflow Just Work.
//!
//! **Before touching `gate_oauth_failure` / `inject_identity_env_with_broker`:**
//! this module is where `SPEC_PROVIDER_ISOLATION_2026_06_20.md`'s INV-A
//! ("never the user's global `~/.<P>` dir") is enforced — or, once already,
//! silently stopped being enforced. Read
//! `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`
//! first. Short version: an unbound oauth-class provider used to
//! auto-route to an AgentMux-owned isolated dir (no user action, no global
//! exposure); a 2026-07-08 refactor orphaned that path without meaning to,
//! and it was never restored — today's gate only chooses between "block"
//! and "true ambient" (`use_ambient_login=true`, zero isolation), not the
//! isolated-auto-provision option that used to exist implicitly.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::{SecretRef, Store};
use crate::backend::wps::{Broker, WaveEvent};

/// Canonical-value enumeration for OAuth-class `IdentityAccount.status`.
///
/// `IdentityAccount.status` is a `String` (free-form) at the SQLite layer
/// — api-key rows keep using whatever the legacy paths wrote
/// (`"unknown"`, `"ok"`, etc.). For oauth-class bindings we pin a small
/// closed set per spec §4.4 so the frontend status-badge dispatch is
/// deterministic and the resolver's expiry probe can never write an
/// off-the-spec string. Every place the resolver SETS or READS an
/// oauth-class status uses these constants.
pub mod oauth_status {
    /// Token file present and (probed) not expired.
    pub const VALID: &str = "valid";
    /// Access token expired; refresh likely succeeds.
    pub const EXPIRED: &str = "expired";
    /// Refresh rejected / file missing / parse error; user must Reconnect.
    pub const NEEDS_REAUTH: &str = "needs_reauth";
    /// Never probed (initial state on bundle import / unprobed provider).
    pub const UNKNOWN: &str = "unknown";
}

/// Result of probing a per-bundle OAuth token directory.
///
/// Computed by [`probe_oauth_status`] reading the CLI's on-disk token
/// file (e.g. `<dir>/.credentials.json` for Claude Code). Maps directly
/// to [`oauth_status`] strings. Returned as an enum so the caller can
/// branch without re-parsing the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProbeStatus {
    Valid,
    Expired,
    NeedsReauth,
}

impl OAuthProbeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => oauth_status::VALID,
            Self::Expired => oauth_status::EXPIRED,
            Self::NeedsReauth => oauth_status::NEEDS_REAUTH,
        }
    }
}

/// Cheap on-disk probe of the per-bundle OAuth token file for a
/// provider. No network calls — just reads + parses the token JSON,
/// then compares `expiresAt` against `now_ms`.
///
/// **Provider token-file shape (spec §4.4 + §4.5):**
/// - `claude` — `<dir>/.credentials.json` with
///   `{ "claudeAiOauth": { "accessToken", "refreshToken", "expiresAt": <ms> } }`
///   (Anthropic's documented format — see
///   `docs/specs/agentmux-isolated-auth.md` §1.6).
/// - `codex` — `<dir>/.credentials.json` (MCP OAuth). Exact field
///   layout undocumented by OpenAI; for now we treat presence-of-file
///   as `Valid` and absence as `NeedsReauth`, deferring strict expiry
///   parsing until the shape is pinned down. Falls through to the
///   Claude parser as a best-effort — if the file is shape-compatible
///   (some CLIs reuse Anthropic's format) the expiry check still works.
/// - `openclaw` — same fallback as codex.
///
/// **Returns** `Some(status)` on a definitive read, `None` when probing
/// isn't supported for the provider (so the caller skips status
/// updates rather than mis-writing `needs_reauth` for a provider whose
/// file we just don't know how to parse yet).
pub fn probe_oauth_status(
    provider: &str,
    dir: &str,
    now_ms: i64,
) -> Option<OAuthProbeStatus> {
    let probe_path: std::path::PathBuf = match provider {
        // Claude Code + codex + openclaw all write to
        // `<config_dir>/.credentials.json` per
        // `docs/specs/provider-auth-isolation.md` (the agentmux-managed
        // dir is what CLAUDE_CONFIG_DIR / CODEX_HOME / OPENCLAW_HOME
        // point at). Codex / openclaw token field-layout is not
        // publicly documented; the parser below treats unrecognised
        // shapes as `Valid` so we don't false-positive a Reconnect on
        // a working session — strict expiry parsing for those two is
        // a follow-up once their JSON is pinned down.
        "claude" | "codex" | "openclaw" => Path::new(dir).join(".credentials.json"),
        _ => return None,
    };

    let contents = match std::fs::read_to_string(&probe_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                error = %e,
                "oauth probe: token file unreadable — status=needs_reauth"
            );
            return Some(OAuthProbeStatus::NeedsReauth);
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                error = %e,
                "oauth probe: token file parse failed — status=needs_reauth"
            );
            return Some(OAuthProbeStatus::NeedsReauth);
        }
    };

    // Claude shape — `claudeAiOauth.expiresAt` is ms since epoch.
    // Many shape-compatible providers nest under the same key; try
    // that first, then fall back to any top-level `expiresAt` /
    // `expires_at` an alternative provider might use.
    let expires_at_ms = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("expiresAt"))
        .and_then(|v| v.as_i64())
        .or_else(|| json.get("expiresAt").and_then(|v| v.as_i64()))
        .or_else(|| json.get("expires_at").and_then(|v| v.as_i64()));

    let has_refresh = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);

    match expires_at_ms {
        Some(exp) if exp <= now_ms => {
            // Past expiry. If a refresh token is present, the next
            // CLI call will likely refresh it cleanly → `expired`
            // (transient, not user-actionable). Without a refresh
            // token the user must re-OAuth → `needs_reauth`.
            if has_refresh {
                Some(OAuthProbeStatus::Expired)
            } else {
                Some(OAuthProbeStatus::NeedsReauth)
            }
        }
        Some(_) => Some(OAuthProbeStatus::Valid),
        None => {
            // Shape doesn't expose an expiry we can parse. Treat the
            // file's existence as `Valid` rather than guess — false
            // `needs_reauth` would force the user to reconnect a
            // working session. codex / openclaw fall here today.
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                "oauth probe: file present but no parseable expiry — status=valid (best-effort)"
            );
            Some(OAuthProbeStatus::Valid)
        }
    }
}

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

    /// The OS keychain read failed (no entry, locked store, or no Secret
    /// Service agent). Armory API keys (`SecretRef::Keychain`) live
    /// in the OS keychain; this surfaces a resolution failure at spawn.
    #[error("keychain error: {0}")]
    KeychainError(String),

    #[error("storage error: {0}")]
    Storage(#[from] StoreError),
}

/// What kind of credential a provider uses, and how
/// `inject_identity_env` puts it into the agent's env at spawn time.
/// Per `specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.3.
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
        // bundles. Originally just claude / codex / openclaw (spec
        // §4.3) — gemini and copilot were added later
        // (REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md
        // §2.5 / §6, Phase C) to close a drift gap: the frontend's
        // `ProviderDefinition` table already marked both
        // `authType: "oauth"`, but this match arm (the actual gate
        // for the spawn-time enforcement AND the per-account
        // isolation-dir minting in identity_handlers.rs, both of
        // which key off this single function) hadn't caught up —
        // meaning neither actually applied to them despite the UI
        // already presenting them as oauth-class. See
        // `oauth_class_matches_frontend_authtype_oauth_set` below,
        // which pins this set staying in sync with the frontend going
        // forward so this doesn't silently drift again.
        "claude" | "codex" | "openclaw" | "gemini" | "copilot" => {
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
/// 3. Read the `db_agent_identity_links` rows for the instance's definition.
/// 4. For each binding: fetch the account, resolve its `SecretRef`,
///    look up the provider's env-var matrix, write each var into
///    `env_vars`.
///    - **Api-key-class** per-binding failures are logged and skipped —
///      other bindings still inject (historical behavior).
///    - **Oauth-class** failures (account row missing, lookup error,
///      non-OAuthConfigDir secret_ref) are BLOCKING unless the agent
///      definition carries `use_ambient_login = 1`: the function returns
///      [`SpawnGateError`] before the CLI process is created. With the
///      opt-in set, the binding is skipped WITHOUT injecting a config dir
///      (the CLI uses its global login) and `identity.spawn.ambient:` is
///      logged at info — the only sanctioned ambient path (spec §2.2).
/// 5. If the agent definition's own provider is oauth-class and no binding
///    for it exists at all (fresh/never-bound or post-delete-cascade), the
///    same gate applies — implicit ambient fallback is how the
///    delete-doesn't-deauthenticate gap happened (spec §2.2 edge case).
///
/// Layer 3 of SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md. Before
/// it, this function was intentionally infallible ("the spawn never aborts
/// because a secret didn't resolve") — that is still true for api-key-class
/// bindings, but oauth-class resolution failures now fail the spawn by
/// default so account deletion is honest at the next spawn.
pub fn inject_identity_env(
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), SpawnGateError> {
    inject_identity_env_with_broker(wstore, id_store, None, block_id, env_vars)
}

/// `inject_identity_env` + optional broker handle so the OAuth-class
/// branch can publish `identityaccounts:changed` on a status change
/// discovered by the expiry probe. The broker is `Option<Arc<Broker>>`
/// — `None` (the legacy entry point, kept for test ergonomics) skips the
/// publish; in production both call sites (`app_api.rs` AgentSendCommand
/// + `websocket.rs` AgentInputCommand) pass `Some(broker.clone())` so any
/// live account list flips its status badge without a reload. Per spec
/// §4.4.
/// Async wrapper around [`inject_identity_env_with_broker`] for use from
/// async spawn handlers. The underlying path does blocking I/O — synchronous
/// SQLite reads and, for `SecretRef::Keychain` accounts, a blocking
/// `keyring` (D-Bus Secret Service on Linux) read — so it runs on a blocking
/// thread via `spawn_blocking` rather than stalling an async runtime worker.
/// Takes ownership of `env_vars` and returns it with identity vars merged in.
///
/// `Err(SpawnGateError)` is the layer-3 spawn gate (see
/// [`inject_identity_env`]): the caller must NOT spawn the CLI and must
/// surface the error on the agent pane like any other spawn failure. A
/// task-join failure also fails CLOSED (`InjectionUnavailable`, blocking
/// the spawn) — see that variant's doc for why an open fallback would
/// systemically bypass the gate after any store panic.
pub async fn inject_identity_env_async(
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    broker: Option<Arc<Broker>>,
    block_id: String,
    env_vars: HashMap<String, String>,
) -> Result<HashMap<String, String>, SpawnGateError> {
    match tokio::task::spawn_blocking(move || {
        let mut env = env_vars;
        inject_identity_env_with_broker(wstore, id_store, broker, &block_id, &mut env)
            .map(|()| env)
    })
    .await
    {
        Ok(result) => result,
        Err(e) => {
            // Fail CLOSED. A join failure means the closure panicked (or
            // was cancelled) — the gate never rendered a verdict, and the
            // panic has likely poisoned the Store mutex, so failing open
            // here would bypass use_ambient_login=false for every later
            // spawn too (reagent P1, PR #2164 round 1). See
            // SpawnGateError::InjectionUnavailable.
            tracing::warn!(
                target: "identity",
                error = %e,
                "identity.spawn.blocked: injection task join failed — failing closed"
            );
            Err(SpawnGateError::InjectionUnavailable { detail: e.to_string() })
        }
    }
}

/// Resolved (provider, account) pair ready for env injection.
///
/// Historically a row of the `db_identity_bindings` table (retired in
/// Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md); now purely
/// an internal shape built from `AgentIdentityLink` rows.
struct IdentityBinding {
    provider: String,
    account_id: String,
}

/// Resolve the identity bindings for an instance from the **direct**
/// agent↔account links only.
///
/// Phase 3 slice 2 PR-B (the flip): the resolver previously dual-read —
/// preferring `db_agent_identity_links` keyed on the instance's DEFINITION
/// id, falling back to the bundle bindings (`db_identity_bindings`) when no
/// direct links existed (PR-A, #1927). That fallback was removed here, and
/// Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md later dropped
/// `db_identity_bundles`/`db_identity_bindings` entirely — every write path
/// (launch flow, per-agent Accounts tab, OAuth Connect/Reconnect) writes a
/// direct link, and the m0013/m0014 migrations (#1927/#1952) backfilled
/// direct links for every pre-existing bundle binding before the drop.
///
/// `broker` publishes `identity:no-direct-links` when the direct set is
/// empty — a standing diagnostic for a non-sentinel identity resolving to
/// zero links (e.g. a stale/migrated-away instance). `None` in tests /
/// call sites that don't have a broker handle.
fn resolve_bindings_for_instance(
    id_store: &Store,
    instance: &crate::backend::storage::store::AgentInstance,
    broker: Option<&Arc<Broker>>,
) -> Vec<IdentityBinding> {
    let direct = id_store
        .agent_identity_list_for_agent(&instance.definition_id)
        .unwrap_or_else(|e| {
            tracing::warn!(
                target: "identity",
                "direct-link lookup failed for definition {}: {}",
                instance.definition_id,
                e,
            );
            Vec::new()
        });

    if direct.is_empty() {
        // Distinct log line + event so a non-sentinel identity resolving
        // to zero links (e.g. an agent that was never linked to an
        // account) is visible, not silently indistinguishable from
        // routine "no accounts configured." Whether the spawn proceeds
        // is decided by the caller's layer-3 definition-provider gate
        // (oauth-class CLI provider + no ambient opt-in → blocked).
        tracing::warn!(
            target: "identity",
            "no direct account links for definition {} (identity {}) — \
             nothing to inject.",
            instance.definition_id,
            instance.identity_id,
        );
        if let Some(b) = broker {
            b.publish(WaveEvent {
                event: "identity:no-direct-links".to_string(),
                scopes: vec![],
                sender: String::new(),
                // Persisted (unlike the rest of this file's ephemeral
                // events) — nothing subscribes live yet, so without
                // persistence a future frontend surface polling via
                // read_event_history would see nothing. Small window;
                // this is a diagnostic, not an audit log.
                persist: 20,
                data: Some(serde_json::json!({
                    "definition_id": instance.definition_id,
                    "identity_id": instance.identity_id,
                })),
            });
        }
    }

    direct
        .into_iter()
        .map(|l| IdentityBinding {
            provider: l.provider,
            account_id: l.account_id,
        })
        .collect()
}

pub fn inject_identity_env_with_broker(
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    broker: Option<Arc<Broker>>,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), SpawnGateError> {
    // Step 1: instance lookup — per-channel, always reads from wstore.
    let instance = match wstore.instance_get_active_for_block(block_id) {
        Ok(Some(i)) => i,
        Ok(None) => {
            // Block has no agent instance row — nothing to inject, and no
            // gating either: quick-launch panes that never went through the
            // launch modal are outside the managed-credentials contract.
            return Ok(());
        }
        Err(e) => {
            tracing::warn!(target: "identity", "instance lookup failed for block {}: {}", block_id, e);
            return Ok(());
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
        // Deliberately NOT gated by layer 3: this sentinel predates the
        // managed-account model and was an explicit "ambient creds"
        // choice at launch time, not a silent fallback.
        tracing::warn!(
            target: "identity",
            "instance {} has empty/blank identity_id — falling back to ambient creds. \
             Legacy row or UI regression?",
            block_id
        );
        return Ok(());
    }

    // Layer-3 gate inputs: the agent definition's ambient opt-in flag and
    // its own CLI provider (the oauth-class provider every launch of this
    // agent uses, whether or not a binding row exists for it). A missing
    // definition row reads as flag=false / no expected provider — the
    // per-binding gate still applies to whatever links exist.
    let (use_ambient, def_provider) = match wstore.agent_def_get(&instance.definition_id) {
        Ok(Some(d)) => (d.use_ambient_login != 0, Some(d.provider)),
        Ok(None) => (false, None),
        Err(e) => {
            tracing::warn!(
                target: "identity",
                "definition lookup failed for {} (layer-3 gate reads use_ambient_login=false): {}",
                instance.definition_id,
                e,
            );
            (false, None)
        }
    };

    // Step 3: bindings — global, reads from id_store. Direct-links-only
    // as of Phase 3 slice 2 PR-B (the flip) — see resolve_bindings_for_instance's
    // doc comment for the transitional gap this closes and the one it
    // doesn't (#1624 PR-C).
    //
    // NOTE: an empty set no longer short-circuits — it falls through to the
    // definition-provider gate below (spec §2.2 edge case: an agent whose
    // oauth-class CLI provider has no binding at all is blocked unless the
    // ambient opt-in is set; the m0017 migration grandfathers pre-existing
    // linkless agents).
    let bindings = resolve_bindings_for_instance(&id_store, &instance, broker.as_ref());

    // Step 4: per-binding resolution + env injection.
    //
    // Each binding's provider determines HOW its account contributes
    // to the agent's env (SPEC_OAUTH_IDENTITY_BUNDLES §4.3):
    //   - ApiKey  — resolve secret_ref to a string, inject as env var(s).
    //   - OAuth   — expect SecretRef::OAuthConfigDir, inject its dir
    //               as the provider's config-dir env var.
    //
    // Api-key-class per-binding failures (unknown provider, account row
    // missing, mismatched secret_ref, secret resolution failed) are logged
    // and skipped — other bindings still inject. Oauth-class failures go
    // through the layer-3 gate: blocking by default, skip-with-
    // `identity.spawn.ambient:` when the agent opted in (spec §2.2).
    let mut injected_oauth: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    // Gate helper for an unresolvable oauth-class binding/provider. Always
    // blocks the spawn — an oauth-class provider must resolve to a real
    // bound IdentityAccount, full stop.
    //
    // `use_ambient_login` used to let this fall through to the user's
    // global/ambient CLI login instead of blocking. That escape hatch is
    // retired (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7,
    // "single point, not global"): a credential the app can't attribute to
    // a specific Armory account is exactly the state that left Marks
    // silently working with an untracked, unrefreshed shared-dir credential
    // and no visible account anywhere in Armory. `use_ambient` is kept as a
    // parameter (read below only for the log line) rather than deleted
    // outright, so the still-live `use_ambient_login` DB column and its
    // callers don't need a synchronized migration to compile — it no longer
    // has any effect on the outcome.
    let gate_oauth_failure = |provider: &str, detail: &str| -> Result<(), SpawnGateError> {
        tracing::warn!(
            target: "identity",
            "identity.spawn.blocked: no credentials for provider {} \
             (definition {}, identity {}) — {}; spawn refused \
             (single-point enforcement — use_ambient_login={}, ignored)",
            provider,
            instance.definition_id,
            instance.identity_id,
            detail,
            use_ambient,
        );
        Err(SpawnGateError::MissingCredentials {
            provider: provider.to_string(),
        })
    };

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
        let is_oauth_class = matches!(class, ProviderClass::OAuth { .. });

        let account = match id_store.identity_get(&binding.account_id) {
            Ok(Some(a)) => a,
            Ok(None) => {
                // The post-delete case (analysis §2.3): the link survived
                // (or was cascaded and re-created stale) but the account
                // row is gone. For oauth-class providers this is the
                // load-bearing layer-3 gate — no more silent fallback to
                // the user's global login.
                if is_oauth_class {
                    gate_oauth_failure(
                        &binding.provider,
                        &format!("account {} row not found", binding.account_id),
                    )?;
                    continue;
                }
                tracing::warn!(
                    target: "identity",
                    "account {} bound to identity {} but row not found — skipping",
                    binding.account_id,
                    instance.identity_id,
                );
                continue;
            }
            Err(e) => {
                if is_oauth_class {
                    gate_oauth_failure(
                        &binding.provider,
                        &format!("account lookup failed for {}: {}", binding.account_id, e),
                    )?;
                    continue;
                }
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
                // Any other variant is a misconfiguration — the account is
                // unresolvable for this provider, so it goes through the
                // layer-3 gate rather than silently leaving the CLI on the
                // user's global login.
                let dir = match &account.secret_ref {
                    SecretRef::OAuthConfigDir { dir } => dir.clone(),
                    other => {
                        gate_oauth_failure(
                            &binding.provider,
                            &format!(
                                "non-OAuthConfigDir secret_ref ({:?}) on account {}",
                                other, binding.account_id,
                            ),
                        )?;
                        continue;
                    }
                };
                env_vars.insert(config_dir_env_var.to_string(), dir.clone());
                injected_oauth.insert(binding.provider.clone());
                tracing::info!(
                    target: "identity",
                    "injected {} for oauth provider {} (identity={}, account={})",
                    config_dir_env_var,
                    binding.provider,
                    instance.identity_id,
                    binding.account_id,
                );

                // Per spec §4.4 — cheap on-disk expiry probe. Reads the
                // CLI's token file inside the bundle dir and refines
                // the IdentityAccount's `status` so the UI can show
                // valid/expired/needs_reauth. Best-effort: probe and
                // upsert failures are logged + ignored (mirrors the
                // per-binding "log + skip" pattern). The probe runs at
                // every spawn but is a single `fs::read_to_string` +
                // JSON parse — negligible overhead.
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if let Some(probed) = probe_oauth_status(&binding.provider, &dir, now_ms) {
                    let new_status = probed.as_str();
                    if account.status != new_status {
                        let mut updated = account.clone();
                        updated.status = new_status.to_string();
                        updated.updated_at = now_ms;
                        match id_store.identity_upsert(&updated) {
                            Ok(()) => {
                                tracing::info!(
                                    target: "identity",
                                    provider = %binding.provider,
                                    account_id = %binding.account_id,
                                    old_status = %account.status,
                                    new_status,
                                    "oauth probe: status updated"
                                );
                                // Publish accounts-changed so any live
                                // account list (Armory Accounts tab,
                                // launch modal) refreshes its Status
                                // column without a reload — the account
                                // row itself is what changed.
                                if let Some(b) = broker.as_ref() {
                                    b.publish(WaveEvent {
                                        event: "identityaccounts:changed".to_string(),
                                        scopes: vec![],
                                        sender: String::new(),
                                        persist: 0,
                                        data: None,
                                    });
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "identity",
                                    provider = %binding.provider,
                                    account_id = %binding.account_id,
                                    error = %e,
                                    "oauth probe: identity_upsert failed — status not persisted",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 5: definition-provider gate (spec §2.2 edge case). The agent's
    // own CLI provider is an oauth-class provider it is "supposed to have
    // credentials for" whether or not a binding row exists — an agent with
    // NO binding for it (never bound, or the link was cascaded away by an
    // account delete) must not silently launch on the user's global login.
    // Bindings that existed but failed were already gated inside the loop,
    // so this only fires for the genuinely-unbound case (no double logs).
    if let Some(p) = def_provider {
        if matches!(provider_class(&p), Some(ProviderClass::OAuth { .. }))
            && !injected_oauth.contains(&p)
            && !bindings.iter().any(|b| b.provider == p)
        {
            gate_oauth_failure(&p, "no account bound for the agent's provider")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{
        AgentInstance, IdentityAccount, InstanceStatus, SecretRef,
    };

    fn make_store() -> Arc<Store> {
        Arc::new(Store::open_in_memory().unwrap())
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

    /// Insert a Block row whose `meta.agentId` points at the agent
    /// def. Phase 3b.4 made `instance_get_active_for_block` resolve
    /// via the block→agent reference (instead of filtering instance
    /// rows by status), so every resolver test that exercises the
    /// inject path needs a real block in the store.
    fn insert_block_for_agent(store: &Store, block_id: &str, agent_id: &str) {
        use crate::backend::obj::{Block, MetaMapType};
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!(agent_id));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
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
        // The known oauth providers must classify as OAuth with the SAME
        // config-dir env vars the CLI provider registry defines (single
        // source of truth). Pinning the expected strings here catches drift
        // in either direction — if the registry changes a value, this test
        // fails and the change becomes deliberate.
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
        assert_eq!(
            provider_class("gemini"),
            Some(ProviderClass::OAuth { config_dir_env_var: "GEMINI_CLI_HOME" }),
        );
        assert_eq!(
            provider_class("copilot"),
            Some(ProviderClass::OAuth { config_dir_env_var: "COPILOT_HOME" }),
        );
    }

    #[test]
    fn oauth_class_matches_frontend_authtype_oauth_set() {
        // REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md §2.5
        // found gemini/copilot marked `authType: "oauth"` in the frontend's
        // `ProviderDefinition` table (frontend/app/view/agent/providers/
        // index.ts) while this function — the actual gate for both the
        // spawn-time enforcement AND the per-account isolation-dir minting
        // — hadn't caught up, so neither mechanism applied to them despite
        // the UI already presenting them as oauth-class. This pins the two
        // sets staying equal going forward. There's no automated cross-
        // language check available, so this list is a manually-maintained
        // mirror of the frontend table — if you add a new `authType:
        // "oauth"` provider there, update FRONTEND_OAUTH_TYPED here too, in
        // the SAME change, not as a follow-up.
        const FRONTEND_OAUTH_TYPED: &[&str] = &["claude", "codex", "gemini", "openclaw", "copilot"];
        const ALL_KNOWN_PROVIDERS: &[&str] = &[
            "claude", "codex", "muxcode", "gemini", "qwen", "openclaw", "kimi", "copilot", "pi",
        ];
        for p in ALL_KNOWN_PROVIDERS {
            let is_oauth_class = matches!(provider_class(p), Some(ProviderClass::OAuth { .. }));
            let is_frontend_oauth_typed = FRONTEND_OAUTH_TYPED.contains(p);
            assert_eq!(
                is_oauth_class, is_frontend_oauth_typed,
                "provider '{p}': backend OAuth-class ({is_oauth_class}) must match \
                 frontend authType:\"oauth\" ({is_frontend_oauth_typed}) — see this \
                 test's doc comment",
            );
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_sets_config_dir_env_var() {
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-claude",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-oauth/claude".to_string(),
            },
        );
        store.identity_upsert(&claude).unwrap();
        store
            .agent_identity_link("def-1", "acct-claude", "claude")
            .unwrap();

        insert_block_for_agent(&store, "block-oauth", "def-1");
        let inst = make_instance("block-oauth", "id-oauth");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        // Spec §2.5 regression: a resolvable oauth account injects
        // unchanged — the layer-3 gate never fires (Ok even with
        // use_ambient_login=0).
        inject_identity_env(store.clone(), store, "block-oauth", &mut env).unwrap();

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
    fn inject_oauth_class_blocks_account_with_non_oauth_secret_ref() {
        // An oauth-class provider (claude) bound to an account whose
        // SecretRef is the API-key shape (Env) is a misconfiguration:
        // the account is unresolvable for the provider, so the layer-3
        // gate blocks the spawn (use_ambient_login=0) instead of
        // mis-injecting the wrong secret or silently launching on the
        // user's global login.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let bad = make_account(
            "acct-bad",
            "claude",
            SecretRef::Env {
                env_var: "CLAUDE_TOKEN_NOT_A_DIR".to_string(),
            },
        );
        store.identity_upsert(&bad).unwrap();
        store
            .agent_identity_link("def-1", "acct-bad", "claude")
            .unwrap();

        insert_block_for_agent(&store, "block-bad", "def-1");
        let inst = make_instance("block-bad", "id-bad");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store, "block-bad", &mut env);

        // Blocking spawn-gate error — and nothing injected.
        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        assert!(env.get("CLAUDE_CONFIG_DIR").is_none());
        assert!(env.is_empty());
    }

    // ── Layer 3 — spawn gating (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4
    //    _2026_07_14.md §2.2/§2.5) ────────────────────────────────────

    /// Fixture def for the gating tests — oauth-class CLI provider
    /// (claude) with a configurable ambient opt-in.
    fn gate_def(use_ambient_login: i64) -> crate::backend::storage::store::AgentDefinition {
        crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login,
        }
    }

    /// Delete an account row while KEEPING its link — the post-delete
    /// shape on a legacy DB (analysis §2.4: real installs got the links
    /// table via ALTER RENAME with no FK cascade clause, so orphan links
    /// are real). FKs are toggled off for the surgical delete so the
    /// test store's DDL-level cascade doesn't fire.
    fn delete_account_row_keep_link(store: &Store, account_id: &str) {
        let conn = store.conn().lock().unwrap();
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys=OFF;
             DELETE FROM db_accounts WHERE id = '{account_id}';
             PRAGMA foreign_keys=ON;"
        ))
        .unwrap();
    }

    #[test]
    fn spawn_blocked_when_bound_oauth_account_missing_and_flag_false() {
        // Spec §2.5 test 1: binding → missing account → flag false →
        // spawn-assembly returns the blocking error (not a skip).
        let store = make_store();
        let mut def = gate_def(0);
        store.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-gone",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-x/claude".to_string(),
            },
        );
        store.identity_upsert(&claude).unwrap();
        store
            .agent_identity_link("def-1", "acct-gone", "claude")
            .unwrap();
        // Delete the account row out from under the link — the exact
        // post-`deleteidentityaccount` shape the resolver used to
        // log-and-skip ("row not found — skipping", analysis §2.3).
        delete_account_row_keep_link(&store, "acct-gone");

        insert_block_for_agent(&store, "block-gate-1", "def-1");
        let inst = make_instance("block-gate-1", "id-x");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store, "block-gate-1", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        // Nothing was injected — the CLI process must never be created.
        assert!(env.is_empty());
        // The error's Display is the user-facing pane message (spec §2.2
        // wording) — pin the load-bearing pieces.
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("no credentials for claude"), "got: {msg}");
        assert!(msg.contains("Bind an account for this provider in the Armory"), "got: {msg}");
    }

    #[test]
    fn spawn_still_blocked_when_bound_oauth_account_missing_and_flag_true() {
        // Was spawn_proceeds_ambient_when_bound_oauth_account_missing_and_flag_true
        // — the ambient opt-in it exercised is retired
        // (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7). A missing
        // account now blocks the spawn unconditionally; `use_ambient_login`
        // no longer changes the outcome. This test pins that the flag is
        // truly inert, not just untested.
        let store = make_store();
        let mut def = gate_def(1);
        store.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-gone-2",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-y/claude".to_string(),
            },
        );
        store.identity_upsert(&claude).unwrap();
        store
            .agent_identity_link("def-1", "acct-gone-2", "claude")
            .unwrap();
        delete_account_row_keep_link(&store, "acct-gone-2");

        insert_block_for_agent(&store, "block-gate-2", "def-1");
        let inst = make_instance("block-gate-2", "id-y");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store, "block-gate-2", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        assert!(env.is_empty());
    }

    #[test]
    fn spawn_blocked_when_oauth_def_provider_has_no_binding_and_flag_false() {
        // Spec §2.2 edge case: an agent whose oauth-class CLI provider has
        // NO binding at all (never bound, or the link was cascaded away by
        // an account delete) is blocked without the opt-in — implicit
        // ambient is how the delete-doesn't-deauthenticate gap happened.
        // (The m0017 migration grandfathers pre-existing linkless agents
        // to flag=1; `inject_no_direct_links_injects_nothing` covers that
        // side.)
        //
        // THIS TEST IS THE INV-A REGRESSION CANARY
        // (`docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`):
        // a completely unconfigured agent (zero bindings, default flag)
        // is the exact shape that used to fall through to the user's
        // global `~/.claude` before commit `e5ab2d09` orphaned the
        // isolated-auto-provision path. If a future refactor makes this
        // assert start failing (env no longer empty, or `res` becomes
        // `Ok`), read that retro before "fixing" the test — the failure
        // is very likely the invariant regressing a second time, not the
        // test being wrong.
        let store = make_store();
        let mut def = gate_def(0);
        store.agent_def_insert(&mut def).unwrap();

        insert_block_for_agent(&store, "block-gate-3", "def-1");
        let inst = make_instance("block-gate-3", "id-z");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store, "block-gate-3", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
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
        inject_identity_env(store.clone(), store, "block-no-instance", &mut env).unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn inject_blank_identity_does_nothing() {
        let store = make_store();
        // Need a definition for the FK on db_agent_instances.
        let mut def = crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        insert_block_for_agent(&store, "block-blank", "def-1");
        let mut inst = make_instance("block-blank", "blank");
        store.instance_create(&inst).unwrap();
        let _ = inst; // keep clippy happy

        let mut env: HashMap<String, String> = HashMap::new();
        // The blank sentinel is the pre-managed-accounts explicit "ambient
        // creds" launch choice — NOT gated by layer 3 (Ok even with
        // use_ambient_login=0).
        inject_identity_env(store.clone(), store, "block-blank", &mut env).unwrap();
        assert!(env.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_full_round_trip_plaintext_dev() {
        let store = make_store();

        // Agent definition.
        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — the layer-3 gate only applies to
            // claude/codex/openclaw definitions, so a non-oauth def here
            // keeps this test focused on the api-key round trip below
            // without a claude account fixture it doesn't otherwise need.
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

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
            .agent_identity_link("def-1", "acct-gh", "github")
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
            .agent_identity_link("def-1", "acct-anth", "anthropic")
            .unwrap();

        // Instance for the block, pointing at id-work.
        insert_block_for_agent(&store, "block-1", "def-1");
        let inst = make_instance("block-1", "id-work");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store, "block-1", &mut env).unwrap();

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
    fn inject_via_direct_link_round_trip() {
        // A DIRECT agent<->account link resolves and injects the
        // account's env vars. Mirrors `inject_full_round_trip_plaintext_dev`.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — see the identical note in
            // inject_full_round_trip_plaintext_dev above.
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let github = make_account(
            "acct-gh",
            "github",
            SecretRef::PlaintextDev {
                plaintext_dev: "ghp_direct".to_string(),
            },
        );
        store.identity_upsert(&github).unwrap();
        store
            .agent_identity_link("def-1", "acct-gh", "github")
            .unwrap();

        insert_block_for_agent(&store, "block-direct", "def-1");
        let inst = make_instance("block-direct", "id-direct");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store, "block-direct", &mut env).unwrap();

        assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_direct"));
        assert_eq!(env.get("GH_TOKEN").map(String::as_str), Some("ghp_direct"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_no_direct_links_injects_nothing() {
        // An instance with a non-sentinel identity_id but no
        // db_agent_identity_links row for its definition — e.g. an
        // account exists but was never linked — injects nothing.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — this test targets the zero-direct-
            // links path itself, which is a distinct concern from the
            // layer-3 oauth gate (covered separately by the spawn_blocked_*
            // tests below).
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let github = make_account(
            "acct-unlinked",
            "github",
            SecretRef::PlaintextDev {
                plaintext_dev: "ghp_unlinked".to_string(),
            },
        );
        store.identity_upsert(&github).unwrap();

        insert_block_for_agent(&store, "block-unlinked", "def-1");
        let inst = make_instance("block-unlinked", "id-unused");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store, "block-unlinked", &mut env).unwrap();

        // Nothing injected — an account with no direct link is invisible
        // to the resolver.
        assert!(env.get("GITHUB_TOKEN").is_none());
        assert!(env.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_no_direct_links_publishes_event() {
        // Same setup as inject_no_direct_links_injects_nothing, but
        // asserts the diagnostic WaveEvent fires — the standing signal
        // for #1624 so a future frontend surface (or just log triage) can
        // see when a non-sentinel identity resolves to zero direct links.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — same rationale as
            // inject_no_direct_links_injects_nothing above.
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let github = make_account(
            "acct-unlinked-2",
            "github",
            SecretRef::PlaintextDev {
                plaintext_dev: "ghp_unlinked_2".to_string(),
            },
        );
        store.identity_upsert(&github).unwrap();

        insert_block_for_agent(&store, "block-unlinked-2", "def-1");
        let inst = make_instance("block-unlinked-2", "id-unused-2");
        store.instance_create(&inst).unwrap();

        let broker = Arc::new(Broker::new());
        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env_with_broker(
            store.clone(),
            store,
            Some(broker.clone()),
            "block-unlinked-2",
            &mut env,
        )
        .unwrap();

        // The event is persisted (see resolve_bindings_for_instance's
        // publish) precisely so it's readable here without needing a live
        // subscriber wired up at publish time.
        let history = broker.read_event_history("identity:no-direct-links", "", 10);
        let event = history.last().expect("expected identity:no-direct-links event");
        assert_eq!(
            event.data.as_ref().and_then(|d| d.get("definition_id")).and_then(|v| v.as_str()),
            Some("def-1"),
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_partial_success_skips_failed_bindings() {
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — this test targets partial-success
            // skip behavior across api-key bindings, unrelated to the
            // layer-3 oauth gate.
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

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
            .agent_identity_link("def-1", "acct-good", "github")
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
            .agent_identity_link("def-1", "acct-bad", "anthropic")
            .unwrap();

        insert_block_for_agent(&store, "block-mixed", "def-1");
        let inst = make_instance("block-mixed", "id-mixed");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store, "block-mixed", &mut env).unwrap();

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

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Not oauth-class (§4.3) — this test targets the unknown-
            // provider skip path, unrelated to the layer-3 oauth gate.
            provider: "kimi".to_string(),
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let custom = make_account(
            "acct-custom",
            "custom",
            SecretRef::PlaintextDev {
                plaintext_dev: "ignored".to_string(),
            },
        );
        store.identity_upsert(&custom).unwrap();
        store
            .agent_identity_link("def-1", "acct-custom", "custom")
            .unwrap();

        insert_block_for_agent(&store, "block-future", "def-1");
        let inst = make_instance("block-future", "id-future");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store, "block-future", &mut env).unwrap();
        // No env-var matrix for "custom" — nothing injected, no panic.
        assert!(env.is_empty());
    }

    // ── PR D — OAuth expiry probe + status semantics ───────────────────

    /// Helper: write a Claude-shape `.credentials.json` into a temp dir
    /// and return the dir path. `expires_ms` controls validity; `with_refresh`
    /// toggles the refreshToken field so the resolver can distinguish
    /// `Expired` (refresh present) from `NeedsReauth` (no refresh).
    fn write_claude_creds(
        dir: &std::path::Path,
        expires_ms: i64,
        with_refresh: bool,
    ) {
        std::fs::create_dir_all(dir).unwrap();
        let body = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "test-access",
                "refreshToken": if with_refresh { "test-refresh" } else { "" },
                "expiresAt": expires_ms,
            }
        });
        std::fs::write(
            dir.join(".credentials.json"),
            serde_json::to_string(&body).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn probe_oauth_status_unknown_provider_returns_none() {
        // Probing a provider that isn't in the oauth-class set is a
        // signal to the caller to leave `status` alone — None ≠
        // NeedsReauth. Guards against silent mis-classification of
        // api-key providers if a future caller accidentally feeds
        // them through here.
        let r = probe_oauth_status("github", "/tmp/whatever", 0);
        assert_eq!(r, None);
    }

    #[test]
    fn probe_oauth_status_missing_dir_is_needs_reauth() {
        let r = probe_oauth_status("claude", "/definitely/does/not/exist-xyz-9q", 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_future_expiry_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms + 3_600_000, true);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::Valid));
    }

    #[test]
    fn probe_oauth_status_past_expiry_with_refresh_is_expired() {
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms - 1, true);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::Expired));
    }

    #[test]
    fn probe_oauth_status_past_expiry_no_refresh_is_needs_reauth() {
        // No refresh token in the file → the CLI can't auto-refresh
        // and the user has to OAuth again. Maps to `needs_reauth`,
        // NOT `expired` (per spec §4.4).
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms - 1, false);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_malformed_json_is_needs_reauth() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".credentials.json"), "{ not json").unwrap();
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_codex_unknown_shape_is_valid_best_effort() {
        // codex / openclaw token-file layouts aren't publicly
        // documented; our parser falls through to "Valid" when the
        // file exists but lacks any parseable expiry. Better than
        // false `needs_reauth` on a working session — strict parsing
        // is a follow-up once the shape is pinned.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"some":"opaque-codex-blob"}"#,
        )
        .unwrap();
        let r = probe_oauth_status("codex", tmp.path().to_str().unwrap(), 0);
        assert_eq!(r, Some(OAuthProbeStatus::Valid));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_probes_and_flips_status_to_needs_reauth() {
        // Full integration: an oauth-class binding pointing at a
        // bundle dir with NO token file → the probe surfaces
        // `needs_reauth` and the resolver upserts the account row
        // with the new status. Spec §4.4.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        // Bundle dir intentionally empty — probe should report
        // needs_reauth (no token file).
        let tmp = tempfile::tempdir().unwrap();
        let bundle_dir = tmp.path().to_str().unwrap().to_string();

        let claude = IdentityAccount {
            id: "acct-claude".to_string(),
            name: "claude-acct-claude".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: bundle_dir },
            context: serde_json::json!({}),
            // Start as "valid" — the probe should flip it to
            // "needs_reauth".
            status: oauth_status::VALID.to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&claude).unwrap();
        store
            .agent_identity_link("def-1", "acct-claude", "claude")
            .unwrap();

        insert_block_for_agent(&store, "block-probe", "def-1");
        let inst = make_instance("block-probe", "id-probe");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), "block-probe", &mut env).unwrap();

        // Env injection still happened (resolver doesn't block on
        // probe outcome — the CLI launches with the dir env var set
        // and will trigger OAuth itself when it sees no tokens).
        assert!(env.get("CLAUDE_CONFIG_DIR").is_some());

        // Status row was UPDATED to needs_reauth.
        let after = store.identity_get("acct-claude").unwrap().unwrap();
        assert_eq!(after.status, oauth_status::NEEDS_REAUTH);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_probe_preserves_status_when_valid() {
        // Future-dated token + status already "valid" → no-op
        // upsert (no spurious updated_at churn). The assertion is
        // that the status remains "valid" — proving the probe
        // didn't misclassify a working session.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
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
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
        };
        store.agent_def_insert(&mut def).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        write_claude_creds(tmp.path(), now_ms + 3_600_000, true);

        let claude = IdentityAccount {
            id: "acct-ok".to_string(),
            name: "claude-acct-ok".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir {
                dir: tmp.path().to_str().unwrap().to_string(),
            },
            context: serde_json::json!({}),
            status: oauth_status::VALID.to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&claude).unwrap();
        store
            .agent_identity_link("def-1", "acct-ok", "claude")
            .unwrap();

        insert_block_for_agent(&store, "block-ok", "def-1");
        let inst = make_instance("block-ok", "id-ok");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), "block-ok", &mut env).unwrap();

        let after = store.identity_get("acct-ok").unwrap().unwrap();
        assert_eq!(after.status, oauth_status::VALID);
        // updated_at unchanged — the resolver only upserts when the
        // probed status differs from the stored value.
        assert_eq!(after.updated_at, 0);
    }
}
