// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `inject_identity_env_with_broker`, one function each, in the order the gate runs
//! them. They stay in the `inject` module tree (a child of `inject.rs`, where the gate and its tests
//! live) so the gate is read and changed as one unit; the INV-A warning on
//! `inject_identity_env_with_broker` covers every step here, `gate_oauth_failure` included.

use super::*;

use std::collections::HashSet;

use crate::backend::storage::store::AgentInstance;

/// A block with no active agent instance: nothing to inject, unless it names a deleted agent.
pub(super) fn check_row_less_block(mstore: &Store, block_id: &str) -> Result<(), SpawnGateError> {
    // #3577: a block that names an agent which was DELETED — its pane
    // survived the delete (another window, or mid-spawn) — is refused,
    // not spawned on the shared login with no identity. Only a
    // positive "deleted" signal refuses; every other row-less block
    // keeps today's behaviour.
    if let Some(agent_id) = block_agent_id(mstore, block_id) {
        if mstore.agent_was_deleted(&agent_id) {
            return Err(SpawnGateError::AgentDeleted { agent_id });
        }
    }
    // Otherwise the block has no agent instance row — nothing to
    // inject, and no gating either: template sessions and quick-launch
    // panes are outside the managed-credentials contract.
    Ok(())
}

/// An empty or "blank" identity_id is no longer a bypass: it is logged and gated like any other.
pub(super) fn warn_on_blank_identity(instance: &AgentInstance, block_id: &str) {
    //
    // #2463 Finding 2: this used to `return Ok(())` here (silent ambient-
    // creds fallback), which bypassed the layer-3 "must have a bound
    // account" gate (steps 4 and 5) entirely — a genuinely brand-new agent launched
    // with no account selected got identity_id="" and spawned on whatever
    // ambient credential happened to be sitting on disk, with nothing
    // bound in Connectors. Observed behavior therefore depended on whether the
    // test machine happened to have a stray credential file, not on any
    // actual policy decision. The UI no longer produces empty/"blank"
    // identity_id for new launches (identity is required at submit-time —
    // SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md), so seeing one here
    // means a legacy continuation row or a UI regression; either way it
    // must be gated the same as any other unresolvable identity_id rather
    // than silently bypassed. `resolve_bindings_for_instance` (step 3) keys
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
}

/// The agent definition's own CLI provider (canonical), and the launch's template parent.
pub(super) fn definition_provider(mstore: &Store, id_store: &Store, instance: &AgentInstance) -> (Option<String>, String) {
    // Layer-3 gate input: the agent definition's own CLI provider (the
    // oauth-class provider every launch of this agent uses, whether or not
    // a binding row exists for it). A missing definition row reads as no
    // expected provider — the per-binding gate still applies to whatever
    // links exist.
    //
    // Provider is resolved via `id_store.resolve_effective_provider_id`
    // (`backend/storage/agents/definition.rs`), the one resolver `agent_open.rs`'s
    // spawn path also uses, so this gate checks credentials for exactly the
    // provider that will run: the agent's own, read-only after creation, or
    // its bundle's when it has none (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md
    // §3.1). One resolver for both call sites so they can't disagree (#2587).
    //
    // Canonicalized via `resolve_provider_alias` (codex P1 on PR #2377): a
    // definition's own `provider` field predates this alias table in rare
    // cases, and this value is compared in steps 4 and 5 against `injected_oauth` /
    // `bindings`' raw (possibly aliased) provider strings — comparing
    // uncanonicalized would let a genuinely-injected alias-only binding
    // still trip the "no account bound" gate. Applied to the bundle-
    // resolved value, not the raw column, so an aliased definition whose
    // bundle already carries the canonical id doesn't get re-aliased
    // incorrectly (resolve_provider_alias is idempotent on an
    // already-canonical id, so this is safe either way).
    match mstore.agent_def_get(&instance.definition_id) {
        Ok(Some(d)) => {
            let effective_provider = id_store.resolve_effective_provider_id(&d);
            let template_parent_id = template_parent_id_if_seeded(mstore, &d.parent_id);
            (
                Some(resolve_provider_alias(&effective_provider).to_string()),
                template_parent_id,
            )
        }
        Ok(None) => (None, String::new()),
        Err(e) => {
            tracing::warn!(
                target: "identity",
                "definition lookup failed for {}: {}",
                instance.definition_id,
                e,
            );
            (None, String::new())
        }
    }
}

// Gate helper for an unresolvable oauth-class binding/provider. Always
// blocks the spawn — an oauth-class provider must resolve to a real
// bound IdentityAccount, full stop.
//
// `use_ambient_login` used to let this fall through to the user's
// global/ambient CLI login instead of blocking. That escape hatch is
// retired (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7,
// "single point, not global"): a credential the app can't attribute to
// a specific account is exactly the state that left Marks
// silently working with an untracked, unrefreshed shared-dir credential
// and no visible account anywhere in Connectors. The flag is no longer read.
pub(super) fn gate_oauth_failure(instance: &AgentInstance, provider: &str, linked: bool, detail: &str) -> Result<(), SpawnGateError> {
    tracing::warn!(
        target: "identity",
        "identity.spawn.blocked: no credentials for provider {} \
         (definition {}, identity {}) — {}; spawn refused \
         (single-point enforcement)",
        provider,
        instance.definition_id,
        instance.identity_id,
        detail,
    );
    Err(SpawnGateError::MissingCredentials {
        provider: provider.to_string(), linked,
    })
}

/// The per-binding step (4): injects each binding's account into the spawn env, and remembers which
/// oauth providers it injected for the unbound-provider gate that follows.
pub(super) struct BindingInjector<'a> {
    pub(super) id_store: &'a Arc<Store>,
    pub(super) identity_store: &'a Arc<Store>,
    pub(super) broker: Option<&'a Arc<Broker>>,
    pub(super) instance: &'a AgentInstance,
    /// Canonical ids of the oauth providers a binding injected.
    pub(super) injected_oauth: HashSet<String>,
}

impl BindingInjector<'_> {
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
    // through the layer-3 gate, which always blocks.
    pub(super) fn inject(&mut self, binding: &IdentityBinding, env_vars: &mut HashMap<String, String>) -> Result<(), SpawnGateError> {
        let class = match provider_class(&binding.provider) {
            Some(c) => c,
            None => {
                tracing::warn!(
                    target: "identity",
                    "no provider class for {} (binding for identity {}) — skipping",
                    binding.provider,
                    self.instance.identity_id,
                );
                return Ok(());
            }
        };
        let is_oauth_class = matches!(class, ProviderClass::OAuth { .. });

        let Some((account, account_store)) = self.bound_account(binding, is_oauth_class)? else {
            return Ok(());
        };

        match class {
            ProviderClass::ApiKey { env_vars: env_keys } => {
                self.inject_api_key(binding, env_keys, &account, env_vars);
                Ok(())
            }
            ProviderClass::OAuth { config_dir_env_var } => {
                self.inject_oauth(binding, config_dir_env_var, &account, &account_store, env_vars)
            }
        }
    }

    /// The binding's account, from this channel's store only (`resolve_account_for_spawn`). `None`
    /// skips a binding that can't be read; an oauth-class one that can't is refused.
    fn bound_account(&self, binding: &IdentityBinding, is_oauth_class: bool) -> Result<Option<(IdentityAccount, Arc<Store>)>, SpawnGateError> {
        // Per-channel ONLY (see `resolve_account_for_spawn`) — a credential
        // authenticated in another channel must not satisfy this spawn.
        match resolve_account_for_spawn(self.id_store, &binding.account_id) {
            Ok(Some(pair)) => Ok(Some(pair)),
            Ok(None) => {
                // The post-delete case (analysis §2.3): the link survived
                // (or was cascaded and re-created stale) but the account
                // row is gone. For oauth-class providers this is the
                // load-bearing layer-3 gate — no more silent fallback to
                // the user's global login.
                if is_oauth_class {
                    gate_oauth_failure(
                        self.instance, &binding.provider, true,
                        &format!("account {} row not found", binding.account_id),
                    )?;
                    return Ok(None);
                }
                tracing::warn!(
                    target: "identity",
                    "account {} bound to identity {} but row not found — skipping",
                    binding.account_id,
                    self.instance.identity_id,
                );
                Ok(None)
            }
            Err(e) => {
                if is_oauth_class {
                    gate_oauth_failure(
                        self.instance, &binding.provider, true,
                        &format!("account lookup failed for {}: {}", binding.account_id, e),
                    )?;
                    return Ok(None);
                }
                tracing::warn!(
                    target: "identity",
                    "account lookup failed for {}: {}",
                    binding.account_id,
                    e,
                );
                Ok(None)
            }
        }
    }

    /// An api-key-class binding: its secret goes into every env var the provider lists.
    /// A secret that can't be resolved is logged and skipped; other bindings still inject.
    fn inject_api_key(&self, binding: &IdentityBinding, env_keys: &[&str], account: &IdentityAccount, env_vars: &mut HashMap<String, String>) {
        let secret = match resolve_account_secret(account, self.id_store, self.identity_store) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    "secret resolution failed for account {} (provider {}): {} — skipping",
                    binding.account_id,
                    binding.provider,
                    e,
                );
                return;
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
            self.instance.identity_id,
            binding.account_id,
        );
    }

    /// An oauth-class binding: the account's config dir goes into the provider's config-dir env var,
    /// after the isolation checks. Anything that can't be resolved is refused by the layer-3 gate.
    fn inject_oauth(
        &mut self,
        binding: &IdentityBinding,
        config_dir_env_var: &str,
        account: &IdentityAccount,
        account_store: &Arc<Store>,
        env_vars: &mut HashMap<String, String>,
    ) -> Result<(), SpawnGateError> {
        // OAuth-class bindings expect SecretRef::OAuthConfigDir.
        // Any other variant is a misconfiguration — the account is
        // unresolvable for this provider, so it goes through the
        // layer-3 gate rather than silently leaving the CLI on the
        // user's global login.
        let dir = match &account.secret_ref {
            SecretRef::OAuthConfigDir { dir } => dir.clone(),
            other => {
                gate_oauth_failure(
                    self.instance, &binding.provider, true,
                    &format!(
                        "non-OAuthConfigDir secret_ref ({:?}) on account {}",
                        other, binding.account_id,
                    ),
                )?;
                return Ok(());
            }
        };
        self.refuse_unisolated_dir(binding, &dir)?;
        env_vars.insert(config_dir_env_var.to_string(), dir.clone());
        // Canonicalized (#2377) — see `definition_provider`;
        // its provider is now canonical too, so
        // this must match it on the same terms.
        self.injected_oauth.insert(resolve_provider_alias(&binding.provider).to_string());
        tracing::info!(
            target: "identity",
            "injected {} for oauth provider {} (identity={}, account={})",
            config_dir_env_var,
            binding.provider,
            self.instance.identity_id,
            binding.account_id,
        );
        self.link_conversation_history(binding, &dir);
        self.refresh_oauth_status(binding, account, account_store, &dir);
        Ok(())
    }

    /// Refuse a config dir that is the provider's ambient home, or whose CLAUDE.md isolation can't be seeded.
    fn refuse_unisolated_dir(&self, binding: &IdentityBinding, dir: &str) -> Result<(), SpawnGateError> {
        // Block unconditionally if this account's config dir IS the
        // provider's literal ambient home (e.g. `~/.claude`) rather
        // than an AgentMux-isolated dir — nothing upstream of this
        // point validates that, and a real, currently-live account
        // configured exactly this way was found in this repo's own
        // data (docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md
        // §8). See docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md.
        if let Some(provider_cfg) =
            crate::backend::providers::get_provider(resolve_provider_alias(&binding.provider))
        {
            if crate::backend::providers::is_provider_ambient_home_dir(provider_cfg, dir) {
                tracing::warn!(
                    target: "identity",
                    "identity.spawn.blocked: account {} for provider {} \
                     resolves to the ambient home dir ({}) — refusing to \
                     spawn (identity {})",
                    binding.account_id,
                    binding.provider,
                    dir,
                    self.instance.identity_id,
                );
                return Err(SpawnGateError::AmbientHomeDirNotAllowed {
                    provider: binding.provider.clone(),
                    dir: dir.to_string(),
                });
            }
            // `dir` is confirmed NOT the ambient home at this point —
            // but a genuinely separate isolated dir can still lack a
            // CLAUDE.md of its own, which Claude Code CLI treats as
            // "fall through to the real $HOME/.claude/CLAUDE.md,"
            // not "no user-level memory file." Idempotent — cheap
            // no-op after the first call. Fail closed (same as the
            // ambient-home-dir check above) rather than warn-and-
            // continue: Codex P1 + ReAgent P1 on PR #2854 — a
            // warn-only failure here would launch the agent with
            // exactly the unprotected condition this fix exists to
            // close. See SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md.
            if let Err(e) =
                crate::backend::providers::seed_claude_md_placeholder_if_missing(provider_cfg, dir)
            {
                tracing::warn!(
                    target: "identity",
                    "identity.spawn.blocked: account {} for provider {} \
                     failed CLAUDE.md isolation seed ({}) — refusing to \
                     spawn (identity {})",
                    binding.account_id,
                    binding.provider,
                    e,
                    self.instance.identity_id,
                );
                return Err(SpawnGateError::ClaudeMdSeedFailed {
                    provider: binding.provider.clone(),
                    dir: dir.to_string(),
                    error: e.to_string(),
                });
            }
            // Soft-fail: a missing setting costs old transcripts or a
            // later compaction, not isolation, so it never blocks a spawn.
            for (what, e) in crate::backend::providers::seed_managed_claude_settings(provider_cfg, dir) {
                let (account, provider) = (&binding.account_id, &binding.provider);
                tracing::warn!(target: "identity", "account {account} for provider {provider}: could not set {what} in {dir} ({e})");
            }
        }
        Ok(())
    }

    /// Re-verify the conversation-history link on every spawn, best-effort and non-blocking.
    fn link_conversation_history(&self, binding: &IdentityBinding, dir: &str) {
        // (#2605) `dir` is read from this
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
                crate::backend::providers::get_provider(resolve_provider_alias(&binding.provider))
            {
                crate::server::identity_auth_dirs::link_history_if_isolated(
                    &paths,
                    std::path::Path::new(dir),
                    provider_cfg,
                    &binding.account_id,
                    "inject_identity_env (spawn)",
                );
            }
        }
    }

    /// Refine the account's status from the CLI's token file on disk. Best-effort: failures are logged.
    fn refresh_oauth_status(&self, binding: &IdentityBinding, account: &IdentityAccount, account_store: &Arc<Store>, dir: &str) {
        // Per spec §4.4 — cheap on-disk expiry probe. Reads the
        // CLI's token file inside the bundle dir and refines
        // the IdentityAccount's `status` so the UI can show
        // valid/expired/needs_reauth. Best-effort: probe and
        // upsert failures are logged + ignored (mirrors the
        // per-binding "log + skip" pattern). The probe runs at
        // every spawn but is a single `fs::read_to_string` +
        // JSON parse — negligible overhead.
        let now_ms = agentmux_common::time::now_ms();
        // Canonicalized (codex P2 on PR #2377): probe_oauth_status
        // only recognizes the canonical "claude"/"codex"/"openclaw"
        // strings — passing a raw alias like "claude-code" always
        // returned None, silently never refreshing an alias-bound
        // account's status or publishing identityaccounts:changed.
        let canonical_provider = resolve_provider_alias(&binding.provider);
        let probed = probe_oauth_status(canonical_provider, dir, now_ms);
        // Self-heal a stale false-positive (Codex P1, PR #2645): the
        // macOS Keychain carve-out in oauth_probe.rs makes a missing
        // token file return `None` for `claude` on macOS instead of
        // `needs_reauth` — but an account probed BEFORE that
        // carve-out existed may already have `needs_reauth`
        // persisted from the old, buggy behavior. A `None` probe
        // never upserts anything, so without this, an already-
        // mislabeled account would stay stuck showing "reconnect"
        // forever, even though this fix landed specifically to stop
        // that. Correcting to "unknown" rather than "valid" — the
        // probe genuinely can't tell on macOS, and a false "valid"
        // would be exactly as dishonest as the "needs_reauth" this
        // is fixing. Scoped tightly to the exact carve-out
        // condition (not "any None"), so an unsupported provider's
        // genuinely-earned needs_reauth (set by some other path)
        // is never touched by this.
        let new_status: Option<&'static str> = match probed {
            Some(p) => Some(p.as_str()),
            None if canonical_provider == "claude"
                && cfg!(target_os = "macos")
                && account.status == oauth_status::NEEDS_REAUTH =>
            {
                Some("unknown")
            }
            None => None,
        };
        if let Some(new_status) = new_status {
            if account.status != new_status {
                let mut updated = account.clone();
                updated.status = new_status.to_string();
                updated.updated_at = now_ms;
                // Write back to whichever store the account was
                // actually resolved from (`bound_account`) —
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
                        // account list (Connectors → Accounts)
                        // refreshes its Status
                        // column without a reload — the account
                        // row itself is what changed.
                        if let Some(b) = self.broker {
                            b.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
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

/// Step 5: the agent's own oauth-class provider needs a bound account even when no binding row exists.
pub(super) fn gate_unbound_provider(
    def_provider: Option<String>,
    injected_oauth: &HashSet<String>,
    bindings: &[IdentityBinding],
    identity_store: &Store,
    instance: &AgentInstance,
    parent_template_id: &str,
) -> Result<(), SpawnGateError> {
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
            // an unrelated reason (already gated in `BindingInjector::inject`) would be
            // misclassified as "no binding at all" here whenever it's
            // stored under an alias.
            && !bindings.iter().any(|b| resolve_provider_alias(&b.provider) == p)
        {
            gate_oauth_failure(instance, &p, refusal_linked(identity_store, &instance.definition_id, parent_template_id, &p), "no account bound for the agent's provider")?;
        }
    }
    Ok(())
}

/// The last check: the agent can be deleted while the gate reads its definition and bindings.
pub(super) fn recheck_agent_not_deleted(mstore: &Store, instance: &AgentInstance) -> Result<(), SpawnGateError> {
    // #3577, re-checked at the gate's own exit: the instance resolved at step 1,
    // but the agent can be deleted while this function reads its definition
    // and bindings — then `def_provider` is `None`, `bindings` is empty, and
    // the earlier steps pass vacuously (#3591). The
    // only caller, `build_persistent_spawn_env`, re-checks again at its own
    // last store read; this keeps the gate correct on its own.
    if mstore.agent_was_deleted(&instance.id) {
        return Err(SpawnGateError::AgentDeleted {
            agent_id: instance.id.clone(),
        });
    }

    Ok(())
}
