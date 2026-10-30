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
//! those ids it falls back to a hash of the provider and email. Both are
//! hashed: the id travels to other machines and into logs, the provider's own
//! ids and the email do not.

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

#[cfg(test)]
#[path = "stable_account_tests.rs"]
mod tests;
