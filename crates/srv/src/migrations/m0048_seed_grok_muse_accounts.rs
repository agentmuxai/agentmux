// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Give the grok and muse agents that already work an account, so none stops launching.
//!
//! Grok Build and Muse Code become OAuth-class (`identity::resolver::provider_class`): an agent
//! needs a bound account, and the spawn gate refuses one without. Until now they ran on the
//! provider's shared default folder (`<shared>/providers/grok`, `<shared>/providers/muse`), which one
//! `grok login` / `muse login` in AgentMux filled for every agent. That login is the one these agents
//! work with, so it is the one to carry over.
//!
//! For each provider whose shared folder holds a login that is good or can be refreshed, this makes
//! ONE account for it (the agents shared one login, so they share one account), copies the folder into
//! the account's own folder under the identities root, and links every agent of that provider that
//! has no account yet. The copy, not the shared folder, is the account's folder: deleting an account
//! removes its own folder, and cleanup never reaches outside the identities root.
//!
//! What it does not do: it never reads `~/.grok` or `~/.config/muse` (the person's own logins outside
//! AgentMux are not these agents' logins, and copying them in is the per-channel isolation bypass
//! removed on 2026-08-31, `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`). An agent with no login
//! in the shared folder was not working and is left to sign in. Idempotent: a second run finds every
//! agent linked.

use std::path::{Path, PathBuf};

use crate::backend::providers::{get_provider, resolve_provider_alias};
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::backend::storage::store::{AgentDefinition, Store};
use crate::identity::resolver::{effective_identity_links, oauth_status, probe_oauth_status, OAuthProbeStatus};
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

const PROVIDERS: [&str; 2] = ["grok", "muse"];

pub struct M0048SeedGrokMuseAccounts;

impl Migration for M0048SeedGrokMuseAccounts {
    fn id(&self) -> &'static str { "0048_seed_grok_muse_accounts" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Make an account from the shared grok / muse login for the agents that already use it"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() || !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let mstore = Store::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("seed_grok_muse_accounts: open mstore: {e}")))?;
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("seed_grok_muse_accounts: open shared store: {e}")))?;
        let defs = mstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("seed_grok_muse_accounts: list agents: {e}")))?;
        for provider in PROVIDERS {
            let agents: Vec<String> = defs
                .iter()
                .filter(|d| resolve_provider_alias(&d.provider) == provider)
                .filter(|d| !has_account(&mstore, &shared, d, provider))
                .map(|d| d.id.clone())
                .collect();
            if agents.is_empty() {
                continue;
            }
            match seed_account(ctx, &mstore, &shared, provider) {
                Ok(Some(account_id)) => {
                    for agent in &agents {
                        if let Err(e) = shared.agent_identity_link(agent, &account_id, provider) {
                            tracing::warn!(agent_id = %agent, provider, error = %e, "seed_grok_muse_accounts: could not link the agent");
                        }
                    }
                    tracing::info!(provider, account_id = %account_id, agents = agents.len(), "seed_grok_muse_accounts: carried the shared login over");
                }
                Ok(None) => tracing::info!(provider, agents = agents.len(), "seed_grok_muse_accounts: no usable shared login, the agents sign in again"),
                Err(e) => tracing::warn!(provider, error = %e, "seed_grok_muse_accounts: left as is"),
            }
        }
        Ok(())
    }
}

/// Whether the spawn gate would find an account for `provider` on this agent: through the links its
/// spawn resolves (its own, or only when it has none its seeded template parent's).
fn has_account(mstore: &Store, shared: &Store, def: &AgentDefinition, provider: &str) -> bool {
    effective_identity_links(mstore, shared, def)
        .map(|links| links.iter().any(|l| resolve_provider_alias(&l.provider) == provider))
        .unwrap_or(false)
}

/// The shared providers folder and the identities root, from the data paths or beside the shared store.
fn roots(ctx: &MigrationContext) -> (PathBuf, PathBuf) {
    if let Some(paths) = agentmux_common::DataPaths::from_env() {
        return (paths.provider_auth_dir(""), paths.identities_dir());
    }
    let shared_dir = ctx.shared_store_path.parent().map(Path::to_path_buf).unwrap_or_default();
    (shared_dir.join("providers"), shared_dir.join("identities"))
}

fn usable(status: Option<OAuthProbeStatus>) -> Option<&'static str> {
    match status? {
        OAuthProbeStatus::Valid => Some(oauth_status::VALID),
        OAuthProbeStatus::Expired => Some(oauth_status::EXPIRED),
        OAuthProbeStatus::NeedsReauth => None,
    }
}

/// Copies the provider's shared folder into a new account folder and records the account.
/// `None` when the shared folder holds no login worth carrying over.
fn seed_account(ctx: &MigrationContext, mstore: &Store, shared: &Store, provider: &str) -> Result<Option<String>, String> {
    let config = get_provider(provider).ok_or_else(|| format!("{provider} is not in the registry"))?;
    let (providers_root, identities_root) = roots(ctx);
    let source = providers_root.join(config.auth_dir_name);
    let now_ms = agentmux_common::time::now_ms();
    let Some(status) = usable(probe_oauth_status(provider, &source.to_string_lossy(), now_ms)) else {
        return Ok(None);
    };
    let account_id = uuid::Uuid::new_v4().to_string();
    let dir = identities_root.join(&account_id).join(config.auth_dir_name);
    if let Err(e) = copy_dir(&source, &dir) {
        let _ = std::fs::remove_dir_all(identities_root.join(&account_id));
        return Err(format!("copy {} to {}: {e}", source.display(), dir.display()));
    }
    let account = IdentityAccount {
        id: account_id.clone(),
        name: format!("{provider}-oauth"),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string_lossy().into_owned() },
        context: serde_json::json!({}),
        status: status.to_string(),
        created_at: now_ms,
        updated_at: now_ms,
    };
    mstore.identity_upsert_with_mirror(shared, &account).map_err(|e| format!("record the account: {e}"))?;
    Ok(Some(account_id))
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "m0048_seed_grok_muse_accounts_tests.rs"]
mod tests;
