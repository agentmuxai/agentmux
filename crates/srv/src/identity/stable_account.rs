// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A signed-in account's id that is the same on every machine
//! (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.4).
//!
//! An account's local id is a UUID made when it was added, and its label
//! (`email ·2`) is numbered per machine, so two machines that signed in to one
//! provider account share neither. The stable id is derived from what the
//! provider says the account is: for Claude, a hash of the OAuth account and
//! organization ids its CLI recorded ([`identity_key_from_oauth_dir`]), so one
//! email in a personal and a Team organization stays two accounts. Without
//! those ids it falls back to a hash of the provider and email.
//!
//! The id is a name, not a secret. It travels to other machines and into logs,
//! so the provider's own ids and the email are hashed out of it; but an email
//! is guessable, so the fallback form can be confirmed against a known address
//! by anyone who sees the id. Two caveats: a sign-in in the provider's ambient
//! home, or one whose `.claude.json` is unreadable, has no provider ids and gets
//! the email form, so one provider account can show both forms on different
//! machines. A matcher across machines treats an email-form id as a weaker
//! claim and compares the email too.

use sha2::{Digest, Sha256};

use crate::backend::storage::identities::{IdentityAccount, SecretRef};

use super::account_email::identity_key_from_oauth_dir;

/// `<provider>:<16 hex>` for `a`, or `None` when it has neither a provider
/// identity nor an email (an API key, say). The provider is the canonical one,
/// so a `claude-code` alias row and a `claude` row of one sign-in agree.
pub fn stable_account_id(a: &IdentityAccount) -> Option<String> {
    let provider = match crate::backend::providers::resolve_provider_alias(&a.provider) {
        "" => a.provider.to_lowercase(),
        c => c.to_string(),
    };
    if let SecretRef::OAuthConfigDir { dir } = &a.secret_ref {
        if let Some(key) = identity_key_from_oauth_dir(&a.provider, dir) {
            return Some(format!("{provider}:{key}"));
        }
    }
    let email = a.context.get("email")?.as_str()?.trim().to_lowercase();
    if email.is_empty() {
        return None;
    }
    let digest = Sha256::digest(format!("{provider}\0email\0{email}").as_bytes());
    Some(format!("{provider}:e{}", &hex::encode(digest)[..15]))
}

/// Whether `id` has the shape [`stable_account_id`] makes: a lower-case provider, a colon and a
/// short token, so a peer cannot put free text where an account id belongs.
pub fn is_stable_id(id: &str) -> bool {
    let Some((provider, token)) = id.split_once(':') else { return false };
    let word = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    word(provider) && word(token)
}

/// For each pane block, the stable id of the account its run is on (`agent:run_account`, else the
/// agent's link): the lookup the LAN agent list reports. The accounts are read once.
pub fn stable_id_of_block<'a>(
    mstore: &'a crate::backend::storage::store::Store,
    id_store: &crate::backend::storage::store::Store,
    identity_store: &'a crate::backend::storage::store::Store,
) -> impl Fn(&str) -> Option<String> + 'a {
    let stable: std::collections::HashMap<String, String> = id_store
        .identity_list(None)
        .unwrap_or_default()
        .iter()
        .filter_map(|a| Some((a.id.clone(), stable_account_id(a)?)))
        .collect();
    move |block_id: &str| {
        let block = mstore.get::<crate::backend::obj::Block>(block_id).ok()??;
        crate::backend::agent_state::run_accounts_of_block(identity_store, &block).into_iter().find_map(|a| stable.get(&a).cloned())
    }
}

#[cfg(test)]
#[path = "stable_account_tests.rs"]
mod tests;
