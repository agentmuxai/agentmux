// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The credential-injection entry points and the layer-3 spawn gate.
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change): `inject_identity_env` / `inject_identity_env_async`
//! (thin public wrappers), `IdentityBinding` + `resolve_bindings_for_instance`
//! (direct-link lookup), and `inject_identity_env_with_broker` — the
//! security-critical credential-injection gate. Every test that exercises
//! `inject_identity_env_with_broker` moved here WITH it, as one atomic unit,
//! per the module split's constraint that this function and its tests must
//! never be separated.
//!
//! **Before touching `gate_oauth_failure` / `inject_identity_env_with_broker`:**
//! see the warning doc comment directly above `inject_identity_env_with_broker`
//! below.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::backend::storage::StoreError;
use crate::backend::wps::{Broker, WaveEvent};

use super::errors::SpawnGateError;
use super::oauth_probe::probe_oauth_status;
use super::provider::{provider_class, ProviderClass};
use super::secret::resolve_secret;

/// Inject identity-derived env vars into the spawn map for a block.
///
/// This is the public entry point called from the CLI-spawn paths
/// (`AgentInputCommand` in websocket.rs and `AgentSendCommand` in
/// app_api.rs). Resolution flow:
///
/// 1. Look up the active `AgentInstance` for this block. If none
///    exists, the caller didn't go through the launch modal — return
///    immediately, no injection.
/// 2. Read its `identity_id`. Empty / "blank" no longer short-circuits to
///    ambient creds (see #2463 — this used to bypass the layer-3 gate
///    entirely, so whether a spawn required a bound account depended on
///    whether a stray ambient credential happened to exist on the test
///    machine). It now falls through to the same steps below, which gate
///    purely on the agent definition's own provider — identical to how a
///    non-empty-but-unresolvable `identity_id` (e.g. a legacy sentinel)
///    already behaved.
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
    identity_store: Arc<Store>,
    block_id: &str,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), SpawnGateError> {
    inject_identity_env_with_broker(wstore, id_store, identity_store, None, block_id, env_vars)
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
    identity_store: Arc<Store>,
    broker: Option<Arc<Broker>>,
    block_id: String,
    env_vars: HashMap<String, String>,
) -> Result<HashMap<String, String>, SpawnGateError> {
    match tokio::task::spawn_blocking(move || {
        let mut env = env_vars;
        inject_identity_env_with_broker(wstore, id_store, identity_store, broker, &block_id, &mut env)
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

/// Resolve an account by id, checking `id_store` first — the
/// authoritative, per-account-isolatable primary, which is what preserves
/// Armory's disposable-test-account isolation guarantee — and falling
/// back to `identity_store`'s read-through mirror
/// (`IDENTITY_STORE_SCHEMA_VERSION` v2,
/// `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` §3.1) only when the
/// primary lookup misses. Returns the account alongside WHICH store it
/// came from, so a caller that later writes a status update (the OAuth
/// expiry probe below) writes it back to the SAME store instead of always
/// assuming `id_store` — writing to the wrong store would silently create
/// a second, never-read copy.
///
/// reagentx P0 review on PR #2632: before this, the link resolved via
/// `identity_store` (always global) but the account it pointed at could
/// still only exist in `id_store`, which is empty on a fresh, isolated
/// channel — the reported continuity bug wasn't actually fixed end to end
/// without this fallback.
pub fn resolve_account(
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    account_id: &str,
) -> Result<Option<(IdentityAccount, Arc<Store>)>, StoreError> {
    if let Some(a) = id_store.identity_get(account_id)? {
        return Ok(Some((a, id_store.clone())));
    }
    if let Some(a) = identity_store.identity_get(account_id)? {
        return Ok(Some((a, identity_store.clone())));
    }
    Ok(None)
}

/// **Before touching `gate_oauth_failure` / `inject_identity_env_with_broker`:**
/// this module is where `SPEC_PROVIDER_ISOLATION_2026_06_20.md`'s INV-A
/// ("never the user's global `~/.<P>` dir") is enforced — or, once already,
/// silently stopped being enforced. Read
/// `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`
/// first. Short version: an unbound oauth-class provider used to
/// auto-route to an AgentMux-owned isolated dir (no user action, no global
/// exposure); a 2026-07-08 refactor orphaned that path without meaning to,
/// and it was never restored — today's gate only chooses between "block"
/// and "true ambient" (`use_ambient_login=true`, zero isolation), not the
/// isolated-auto-provision option that used to exist implicitly.
pub fn inject_identity_env_with_broker(
    wstore: Arc<Store>,
    id_store: Arc<Store>,
    identity_store: Arc<Store>,
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
    //
    // #2463 Finding 2: this used to `return Ok(())` here (silent ambient-
    // creds fallback), which bypassed the layer-3 "must have a bound
    // account" gate below entirely — a genuinely brand-new agent launched
    // with no account selected got identity_id="" and spawned on whatever
    // ambient credential happened to be sitting on disk, with nothing
    // bound in Armory. Observed behavior therefore depended on whether the
    // test machine happened to have a stray credential file, not on any
    // actual policy decision. The UI no longer produces empty/"blank"
    // identity_id for new launches (identity is required at submit-time —
    // SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md), so seeing one here
    // means a legacy continuation row or a UI regression; either way it
    // must be gated the same as any other unresolvable identity_id rather
    // than silently bypassed. `resolve_bindings_for_instance` below keys
    // strictly off `instance.definition_id`, not `identity_id`, so falling
    // through here is safe regardless of what identity_id contains.
    if instance.identity_id.is_empty() || instance.identity_id == "blank" {
        tracing::warn!(
            target: "identity",
            "instance {} has empty/blank identity_id — falling through to the \
             layer-3 gate instead of ambient creds. Legacy row or UI regression?",
            block_id
        );
    }

    // Layer-3 gate inputs: the agent definition's ambient opt-in flag and
    // its own CLI provider (the oauth-class provider every launch of this
    // agent uses, whether or not a binding row exists for it). A missing
    // definition row reads as flag=false / no expected provider — the
    // per-binding gate still applies to whatever links exist.
    //
    // Provider is resolved via `id_store.resolve_effective_provider_id`
    // (`backend/storage/agents.rs`), NOT `d.provider` directly — the
    // definition's own column can drift post-creation (`agent.define`'s
    // `if_exists=update` path) while the agent's bound ABF bundle's copy
    // is backend-enforced immutable, so the bundle is the one to trust
    // for a security-relevant decision like this gate. Found during a
    // follow-up scoping pass after `agent_open.rs`'s spawn path was fixed
    // for the identical bug (ReAgent review, PR #2587 round 3) — the same
    // resolution logic is now shared between both call sites specifically
    // so they can't drift on it independently again.
    //
    // Canonicalized via `resolve_provider_alias` (codex P1 on PR #2377): a
    // definition's own `provider` field predates this alias table in rare
    // cases, and this value is compared below against `injected_oauth` /
    // `bindings`' raw (possibly aliased) provider strings — comparing
    // uncanonicalized would let a genuinely-injected alias-only binding
    // still trip the "no account bound" gate. Applied to the bundle-
    // resolved value, not the raw column, so an aliased definition whose
    // bundle already carries the canonical id doesn't get re-aliased
    // incorrectly (resolve_provider_alias is idempotent on an
    // already-canonical id, so this is safe either way).
    let (use_ambient, def_provider) = match wstore.agent_def_get(&instance.definition_id) {
        Ok(Some(d)) => {
            let effective_provider = id_store.resolve_effective_provider_id(&d);
            (d.use_ambient_login != 0, Some(resolve_provider_alias(&effective_provider).to_string()))
        }
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
    let bindings = resolve_bindings_for_instance(&identity_store, &instance, broker.as_ref());

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

        let (account, account_store) = match resolve_account(&id_store, &identity_store, &binding.account_id) {
            Ok(Some(pair)) => pair,
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
                // Canonicalized (codex P1 on PR #2377) — see def_provider's
                // doc comment above; def_provider is now canonical too, so
                // this must match it on the same terms.
                injected_oauth.insert(resolve_provider_alias(&binding.provider).to_string());
                tracing::info!(
                    target: "identity",
                    "injected {} for oauth provider {} (identity={}, account={})",
                    config_dir_env_var,
                    binding.provider,
                    instance.identity_id,
                    binding.account_id,
                );

                // reagentx P1 on PR #2605: `dir` above is read from this
                // account's PREVIOUSLY STORED SecretRef::OAuthConfigDir —
                // this is the ordinary spawn path, not `auth.start`, which
                // is the only place that used to (re-)create the history
                // link. An account provisioned before this whole feature
                // shipped, or since its last `auth.start`, would otherwise
                // never get linked at all. Re-verify/re-create it on every
                // spawn instead, same best-effort, non-blocking guarantee
                // as the rest of this function.
                if let Some(paths) = agentmux_common::DataPaths::from_env() {
                    if let Some(provider_cfg) =
                        crate::backend::providers::get_provider(&resolve_provider_alias(&binding.provider))
                    {
                        crate::server::identity_auth_dirs::link_history_if_isolated(
                            &paths,
                            std::path::Path::new(&dir),
                            provider_cfg,
                            &binding.account_id,
                            "inject_identity_env (spawn)",
                        );
                    }
                }

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
                // Canonicalized (codex P2 on PR #2377): probe_oauth_status
                // only recognizes the canonical "claude"/"codex"/"openclaw"
                // strings — passing a raw alias like "claude-code" always
                // returned None, silently never refreshing an alias-bound
                // account's status or publishing identityaccounts:changed.
                if let Some(probed) = probe_oauth_status(resolve_provider_alias(&binding.provider), &dir, now_ms) {
                    let new_status = probed.as_str();
                    if account.status != new_status {
                        let mut updated = account.clone();
                        updated.status = new_status.to_string();
                        updated.updated_at = now_ms;
                        // Write back to whichever store the account was
                        // actually resolved from (resolve_account above) —
                        // not always id_store, since a fallback-mirror hit
                        // must update the mirror, not silently create a
                        // second, never-read copy in id_store.
                        match account_store.identity_upsert(&updated) {
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
            // Canonicalized (codex P1 on PR #2377): `b.provider` is the raw
            // DB value and may still be a legacy alias even though `p` is
            // canonical — without this, a binding that failed injection for
            // an unrelated reason (already gated above) would be
            // misclassified as "no binding at all" here whenever it's
            // stored under an alias.
            && !bindings.iter().any(|b| resolve_provider_alias(&b.provider) == p)
        {
            gate_oauth_failure(&p, "no account bound for the agent's provider")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::oauth_probe::oauth_status;
    use crate::backend::storage::store::{
        AgentInstance, IdentityAccount, InstanceStatus, Memory, SecretRef,
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-oauth", &mut env).unwrap();

        // OAuth dispatch sets the provider's config-dir env var.
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-oauth/claude"),
        );
        // And does NOT set the anthropic api-key env var — dispatch
        // is by provider class, not by token shape.
        assert!(env.get("ANTHROPIC_API_KEY").is_none());
    }

    /// The exact end-to-end scenario reagentx's P0 review on PR #2632
    /// caught: after a version/channel switch, the LINK resolves via
    /// `identity_store` (always global), but if the ACCOUNT row the link
    /// points at only lives in `identity_store`'s fallback mirror — not
    /// `id_store`, which is a fresh, empty, per-channel-isolated store on
    /// the new channel — the spawn must still succeed by falling back to
    /// the mirror. Before `resolve_account`'s fallback, this reproduced the
    /// exact reported bug (spawn refused with "account row not found") even
    /// though the link itself was already fixed.
    #[cfg(debug_assertions)]
    #[test]
    fn inject_resolves_account_via_identity_store_fallback_when_id_store_lacks_it() {
        let wstore = make_store();
        // Deliberately EMPTY of the account and link — simulates a fresh,
        // isolated per-channel id_store on a new channel/version.
        let id_store = make_store();
        // Real identity-store schema (no account_id/agent_id FK, matching
        // production's Store::open_identity_store — NOT open_in_memory's
        // channel schema, which still has both FKs and would reject this
        // link since "def-1" only exists in wstore's own definitions
        // table, a different physical store). Simulates the post-migration
        // state: both the link AND the account mirror row live here,
        // exactly what m0022_identity_store_links_backfill produces.
        let identity_store_tmp = tempfile::NamedTempFile::new().unwrap();
        let identity_store = Arc::new(Store::open_identity_store(identity_store_tmp.path()).unwrap());

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        wstore.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-migrated",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-migrated/claude".to_string(),
            },
        );
        identity_store.identity_upsert(&claude).unwrap();
        identity_store
            .agent_identity_link("def-1", "acct-migrated", "claude")
            .unwrap();

        insert_block_for_agent(&wstore, "block-continuing", "def-1");
        let inst = make_instance("block-continuing", "id-continuing");
        wstore.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(wstore, id_store, identity_store, "block-continuing", &mut env);

        assert!(
            res.is_ok(),
            "spawn must succeed via the identity_store fallback mirror, not fail with \
             'account row not found' — this is the exact reported continuity bug: {res:?}"
        );
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-migrated/claude"),
        );
    }

    /// Reagentx round-3 P0 on PR #2632: the fallback mirror above only
    /// gets populated by the one-time migration backfill unless every live
    /// account-write path also dual-writes via `identity_upsert_with_mirror`
    /// — otherwise an account created (or updated) AFTER this PR shipped
    /// would still dead-end on its own next channel switch, same as before
    /// the fix. This test writes the account through
    /// `identity_upsert_with_mirror` (the same call every live write path —
    /// `identity.account.upsert`, OAuth persist, etc. — now uses) instead of
    /// seeding `identity_store` directly, then simulates a channel switch
    /// with a brand-new, empty `id_store` and confirms resolution still
    /// succeeds via the mirror `identity_upsert_with_mirror` wrote.
    #[cfg(debug_assertions)]
    #[test]
    fn inject_resolves_a_freshly_created_account_after_a_channel_switch_when_written_via_the_mirror_helper() {
        let wstore = make_store();
        let id_store = make_store();
        let identity_store_tmp = tempfile::NamedTempFile::new().unwrap();
        let identity_store = Arc::new(Store::open_identity_store(identity_store_tmp.path()).unwrap());

        // `make_instance` below hardcodes `definition_id: "def-1"` — match it
        // rather than parameterizing a helper shared with other tests.
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        wstore.agent_def_insert(&mut def).unwrap();

        // "Before the channel switch": the account is created live, via the
        // same helper `identity.account.upsert`/OAuth persist use — writing
        // to id_store (the then-current, then-primary store) AND dual-
        // writing the mirror into identity_store.
        let claude = make_account(
            "acct-fresh",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-fresh/claude".to_string(),
            },
        );
        id_store.identity_upsert_with_mirror(&identity_store, &claude).unwrap();
        identity_store
            .agent_identity_link("def-1", "acct-fresh", "claude")
            .unwrap();

        insert_block_for_agent(&wstore, "block-fresh", "def-1");
        let inst = make_instance("block-fresh", "id-fresh");
        wstore.instance_create(&inst).unwrap();

        // "After the channel switch": a brand-new, empty id_store — the
        // account row created above must be unreachable here, exactly like
        // a fresh per-channel-isolated store on a new channel.
        let id_store_after_switch = make_store();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(wstore, id_store_after_switch, identity_store, "block-fresh", &mut env);

        assert!(
            res.is_ok(),
            "a freshly-created account (written via identity_upsert_with_mirror, not migration \
             backfill) must still resolve after a channel switch: {res:?}"
        );
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-fresh/claude"),
        );
    }

    // reagentx P1 on PR #2605: this is the ordinary spawn path (not
    // `auth.start`) — it reads a PREVIOUSLY STORED OAuthConfigDir, which
    // is exactly the case an account provisioned before the history-link
    // feature existed (or since its last `auth.start`) is in. Proves the
    // link now gets (re-)created here too, not only at auth.start.
    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_links_conversation_history_on_the_ordinary_spawn_path() {
        let _lock = crate::test_support::ISOLATED_AUTH_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        // A credential dir that ALREADY exists on disk, under the isolated
        // (per-channel) tree, with real pre-existing history in it — the
        // account was provisioned (auth.start ran) before this test's
        // spawn-time call, exactly like a real already-connected account.
        let isolated_dir = paths.instance_dir.join("identities").join("acct-claude").join("claude");
        std::fs::create_dir_all(isolated_dir.join("projects")).unwrap();
        std::fs::write(
            isolated_dir.join("projects").join("existing-session.jsonl"),
            b"pre-existing history",
        )
        .unwrap();

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-claude",
            "claude",
            SecretRef::OAuthConfigDir { dir: isolated_dir.to_string_lossy().to_string() },
        );
        store.identity_upsert(&claude).unwrap();
        store.agent_identity_link("def-1", "acct-claude", "claude").unwrap();
        insert_block_for_agent(&store, "block-oauth-history", "def-1");
        let inst = make_instance("block-oauth-history", "id-oauth");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), store, "block-oauth-history", &mut env).unwrap();

        let global_history = paths.identity_history_dir("acct-claude", "claude", "projects");
        let via_link = std::fs::read(global_history.join("existing-session.jsonl"))
            .expect("pre-existing history must have been migrated to the global location by the spawn-time link");
        assert_eq!(via_link, b"pre-existing history");

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-bad", &mut env);

        // Blocking spawn-gate error — and nothing injected.
        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        assert!(env.get("CLAUDE_CONFIG_DIR").is_none());
        assert!(env.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_succeeds_when_the_only_binding_is_under_a_legacy_alias() {
        // codex P1 on PR #2377: a definition's own `provider` is canonical
        // ("claude"), but its only db_agent_identity_links row is still
        // under a legacy alias ("claude-code") — carried forward from
        // before providers.rs's alias table existed, or a definition/link
        // pair that otherwise drifted. The binding injects successfully
        // (provider_class already resolves aliases), but before this fix
        // the def-provider gate compared the raw `injected_oauth`/binding
        // provider strings against the canonical def_provider and treated
        // the alias-bound account as "no account bound at all" — blocking
        // a spawn that had already successfully injected valid credentials
        // moments earlier in the same function.
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let claude = make_account(
            "acct-alias",
            "claude-code",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-alias/claude".to_string(),
            },
        );
        store.identity_upsert(&claude).unwrap();
        // Bound under the alias, not the canonical "claude".
        store
            .agent_identity_link("def-1", "acct-alias", "claude-code")
            .unwrap();

        insert_block_for_agent(&store, "block-alias", "def-1");
        let inst = make_instance("block-alias", "id-alias");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-alias", &mut env);

        assert!(res.is_ok(), "expected Ok, got {res:?}");
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-alias/claude"),
        );
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-gate-1", &mut env);

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
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-gate-2", &mut env);

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
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-gate-3", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        assert!(env.is_empty());
    }

    #[test]
    fn inject_no_instance_does_nothing() {
        let store = make_store();
        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), store, "block-no-instance", &mut env).unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn inject_blank_identity_is_gated_same_as_any_other_unresolvable_identity() {
        // #2463 Finding 2: this test used to assert the OPPOSITE — that a
        // blank identity_id was NOT gated by layer 3 and silently fell
        // back to ambient creds even with use_ambient_login=0. That let a
        // brand-new agent launch with no account selected spawn on
        // whatever ambient credential happened to be on disk, with
        // nothing bound in Armory — observably different behavior on two
        // machines running the identical code, purely as a function of
        // which one had a stray credential file. A blank/empty
        // identity_id must be gated exactly like any other unresolvable
        // one (see `spawn_blocked_when_oauth_def_provider_has_no_binding_
        // and_flag_false`, the non-blank equivalent of this test).
        let store = make_store();
        let mut def = gate_def(0);
        store.agent_def_insert(&mut def).unwrap();

        insert_block_for_agent(&store, "block-blank", "def-1");
        let inst = make_instance("block-blank", "blank");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-blank", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
        assert!(env.is_empty());
    }

    #[test]
    fn inject_empty_identity_is_gated_same_as_blank() {
        // Same as the blank case above, but the genuinely-empty string —
        // what a real brand-new-agent launch with no account selected
        // actually produces (#2463 Finding 2's exact repro), as opposed to
        // the legacy "blank" singleton literal.
        let store = make_store();
        let mut def = gate_def(0);
        store.agent_def_insert(&mut def).unwrap();

        insert_block_for_agent(&store, "block-empty", "def-1");
        let inst = make_instance("block-empty", "");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(store.clone(), store.clone(), store, "block-empty", &mut env);

        assert_eq!(
            res,
            Err(SpawnGateError::MissingCredentials { provider: "claude".to_string() }),
        );
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
            // Not oauth-class — the layer-3 gate only applies to providers
            // `provider_class` classifies as OAuth-class (claude / codex /
            // openclaw / gemini / copilot as of
            // REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md
            // §2.5/§6), so a non-oauth def here keeps this test focused on
            // the api-key round trip below without a claude account fixture
            // it doesn't otherwise need.
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-1", &mut env).unwrap();

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-direct", &mut env).unwrap();

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-unlinked", &mut env).unwrap();

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-mixed", &mut env).unwrap();

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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store, "block-future", &mut env).unwrap();
        // No env-var matrix for "custom" — nothing injected, no panic.
        assert!(env.is_empty());
    }

    /// Helper: write a Claude-shape `.credentials.json` into a temp dir
    /// and return the dir path. `expires_ms` controls validity; `with_refresh`
    /// toggles the refreshToken field so the resolver can distinguish
    /// `Expired` (refresh present) from `NeedsReauth` (no refresh).
    ///
    /// Duplicated from `oauth_probe::tests` (same helper, different file)
    /// rather than shared cross-module — kept the modularization split a
    /// pure relocation with no new cross-file test-only visibility surface.
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

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_probes_and_flips_status_to_needs_reauth() {
        // Full integration: an oauth-class binding pointing at a
        // bundle dir with NO token file → the probe surfaces
        // `needs_reauth` and the resolver upserts the account row
        // with the new status. Spec §4.4.
        //
        // Uses "codex", not "claude": the macOS Keychain carve-out added
        // in oauth_probe.rs (retro-macos-keychain-credential-isolation-gap-
        // 2026-08-17.md) makes a missing token file for "claude" return
        // `None` (no status change) on macOS specifically, since Claude
        // Code never writes `.credentials.json` there regardless of the
        // config dir. "codex" isn't covered by that carve-out, so it keeps
        // this test's original, platform-independent needs_reauth signal.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "codex".to_string(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        // Bundle dir intentionally empty — probe should report
        // needs_reauth (no token file).
        let tmp = tempfile::tempdir().unwrap();
        let bundle_dir = tmp.path().to_str().unwrap().to_string();

        let codex_acct = IdentityAccount {
            id: "acct-codex".to_string(),
            name: "codex-acct-codex".to_string(),
            provider: "codex".to_string(),
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
        store.identity_upsert(&codex_acct).unwrap();
        store
            .agent_identity_link("def-1", "acct-codex", "codex")
            .unwrap();

        insert_block_for_agent(&store, "block-probe", "def-1");
        let inst = make_instance("block-probe", "id-probe");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), store.clone(), "block-probe", &mut env).unwrap();

        // Env injection still happened (resolver doesn't block on
        // probe outcome — the CLI launches with the dir env var set
        // and will trigger OAuth itself when it sees no tokens).
        assert!(env.get("CODEX_HOME").is_some());

        // Status row was UPDATED to needs_reauth.
        let after = store.identity_get("acct-codex").unwrap().unwrap();
        assert_eq!(after.status, oauth_status::NEEDS_REAUTH);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn inject_oauth_class_probe_canonicalizes_a_legacy_alias_binding() {
        // codex P2 on PR #2377 (third round): probe_oauth_status only
        // recognizes canonical provider strings ("claude"/"codex"/
        // "openclaw") — passing the raw alias ("claude-code") a
        // migrated/legacy binding may still carry always returned None,
        // silently never refreshing that account's status. Same fixture
        // as inject_oauth_class_probes_and_flips_status_to_needs_reauth
        // above, but bound under the alias instead of the canonical id.
        //
        // Uses "codex-cli" → "codex", not "claude-code" → "claude": the
        // macOS Keychain carve-out (oauth_probe.rs) means a missing token
        // file for "claude" returns `None` on macOS regardless of whether
        // canonicalization ran, which would make this test unable to
        // distinguish "canonicalization is broken" from "hit the carve-out"
        // on that platform. "codex" isn't covered by the carve-out, so it
        // keeps the original, platform-independent needs_reauth signal.
        let store = make_store();

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            provider: "codex".to_string(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let bundle_dir = tmp.path().to_str().unwrap().to_string();

        let codex_acct = IdentityAccount {
            id: "acct-alias".to_string(),
            name: "codex-cli-acct-alias".to_string(),
            provider: "codex-cli".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: bundle_dir },
            context: serde_json::json!({}),
            status: oauth_status::VALID.to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&codex_acct).unwrap();
        // Bound under the alias, not the canonical "codex".
        store
            .agent_identity_link("def-1", "acct-alias", "codex-cli")
            .unwrap();

        insert_block_for_agent(&store, "block-probe-alias", "def-1");
        let inst = make_instance("block-probe-alias", "id-probe-alias");
        store.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        inject_identity_env(store.clone(), store.clone(), store.clone(), "block-probe-alias", &mut env).unwrap();

        assert!(env.get("CODEX_HOME").is_some());
        // Status row was UPDATED to needs_reauth — proves the probe ran
        // with the canonicalized provider id, not the raw alias (which
        // would have returned None and left status untouched at "valid").
        let after = store.identity_get("acct-alias").unwrap().unwrap();
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        inject_identity_env(store.clone(), store.clone(), store.clone(), "block-ok", &mut env).unwrap();

        let after = store.identity_get("acct-ok").unwrap().unwrap();
        assert_eq!(after.status, oauth_status::VALID);
        // updated_at unchanged — the resolver only upserts when the
        // probed status differs from the stored value.
        assert_eq!(after.updated_at, 0);
    }

    // P0 regression test (found in a follow-up scoping pass after
    // agent_open.rs's identical bug was fixed in PR #2587 round 3):
    // the layer-3 gate must resolve the definition's provider through
    // its bound ABF bundle (`id_store.resolve_effective_provider_id`),
    // not `d.provider` directly. Uses two genuinely separate stores —
    // wstore (definition, block, instance) and id_store (bundle,
    // account, link) — so the test can't pass by accident the way it
    // would if both roles were backed by the same `Arc<Store>`, which
    // every OTHER test in this module does (matching production's
    // AppState.wstore/id_store split, but meaning none of them could
    // have caught this class of bug).
    //
    // Scenario: the definition's own `provider` column has drifted to
    // "codex" (e.g. via a since-superseded agent.define update), but
    // its bound bundle's copy — the backend-enforced-immutable one —
    // still correctly says "claude", and the agent has a perfectly
    // valid Claude OAuth account bound. Before the fix, the gate read
    // `d.provider` directly, saw "codex", found no binding for codex
    // (only claude), and WRONGLY BLOCKED a correctly-configured agent's
    // spawn with "no account bound for the agent's provider" — even
    // though a valid claude credential was right there. After the fix,
    // it resolves "claude" via the bundle, the claude binding injects
    // successfully, and the spawn proceeds.
    #[test]
    fn spawn_gate_resolves_provider_through_the_bound_bundle_not_the_drifted_definition_column() {
        let wstore = make_store();
        // Real shared-store schema, not open_in_memory's channel schema —
        // db_agent_identity_links only has a `db_agent_definitions` FK in
        // the channel schema (migrations.rs:334); the shared-store schema
        // (migrations.rs:829) deliberately omits it, since agent
        // definitions never live in the shared store. Using two
        // open_in_memory() stores here would incorrectly enforce an FK
        // that doesn't exist in production's real id_store.
        let id_store_tmp = tempfile::NamedTempFile::new().unwrap();
        let id_store = Arc::new(Store::open_shared(id_store_tmp.path()).unwrap());
        // Same reasoning as id_store above, for the new always-global
        // identity store (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md):
        // open_identity_store's real schema (no db_accounts FK at all — see
        // its own doc comment) rather than open_in_memory's channel schema.
        let identity_store_tmp = tempfile::NamedTempFile::new().unwrap();
        let identity_store = Arc::new(Store::open_identity_store(identity_store_tmp.path()).unwrap());

        let mut def = crate::backend::storage::store::AgentDefinition {
            id: "def-1".to_string(),
            slug: String::new(),
            name: "T".to_string(),
            icon: "✦".to_string(),
            // Drifted: the definition's own column says codex...
            provider: "codex".to_string(),
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
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            // ...but its bound bundle (below, in id_store) says claude.
            memory_id: "bundle-1".to_string(),
        };
        wstore.agent_def_insert(&mut def).unwrap();

        let bundle = Memory {
            id: "bundle-1".to_string(),
            name: "Drift Test Bundle".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: "claude".to_string(),
            model: "anthropic".to_string(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        id_store.bundle_memory_upsert(&bundle).unwrap();

        let claude = make_account(
            "acct-drift",
            "claude",
            SecretRef::OAuthConfigDir {
                dir: "/var/agentmux/identities/id-drift/claude".to_string(),
            },
        );
        id_store.identity_upsert_with_mirror(&identity_store, &claude).unwrap();
        identity_store
            .agent_identity_link("def-1", "acct-drift", "claude")
            .unwrap();

        insert_block_for_agent(&wstore, "block-drift", "def-1");
        let inst = make_instance("block-drift", "id-drift");
        wstore.instance_create(&inst).unwrap();

        let mut env: HashMap<String, String> = HashMap::new();
        let res = inject_identity_env(wstore, id_store, identity_store, "block-drift", &mut env);

        assert!(res.is_ok(), "a valid claude account must not be blocked by a stale codex column: {res:?}");
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/var/agentmux/identities/id-drift/claude"),
            "the claude binding must actually inject, proving the gate expected claude (from the bundle), not codex (from the drifted column)"
        );
    }
}
