// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OS-native secret storage for Armory API keys.
//!
//! Backs `SecretRef::Keychain { service, account }`. The plaintext key
//! lives only in the OS keychain (macOS Keychain / Windows Credential
//! Manager / Linux Secret Service via the `keyring` crate); the DB holds
//! just the pointer plus non-secret metadata + a masked tail. Reads return
//! a `Zeroizing<String>` so the plaintext is wiped from memory on drop.
//!
//! See specs/SPEC_TRUST_CENTER_2026_06_15.md §7 (best practices) and §12.2.
//!
//! NOTE: an encrypted-file fallback for headless Linux without a Secret
//! Service agent is a documented follow-up (spec §12.2); the desktop app
//! ships with a keychain on all three platforms, so keyring is the path
//! here. Failures surface as a typed error rather than a silent downgrade.

use keyring::Entry;
use zeroize::Zeroizing;

/// Keychain service string — constant across all AgentMux secrets.
pub const SERVICE: &str = "agentmux";

/// Build the keychain account string for an identity-account id.
pub fn account_key(account_id: &str) -> String {
    format!("acct:{account_id}")
}

fn entry(account_id: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, &account_key(account_id))
        .map_err(|e| format!("keychain entry init failed: {e}"))
}

/// Store (or overwrite) the secret for `account_id` in the OS keychain.
pub fn put(account_id: &str, secret: &str) -> Result<(), String> {
    entry(account_id)?
        .set_password(secret)
        .map_err(|e| format!("keychain write failed: {e}"))
}

/// Read the secret for `account_id`. Returned wrapped in `Zeroizing` so it
/// is wiped on drop. Resolved at agent spawn time when injecting env vars.
pub fn get(account_id: &str) -> Result<Zeroizing<String>, String> {
    let pw = entry(account_id)?
        .get_password()
        .map_err(|e| format!("keychain read failed: {e}"))?;
    Ok(Zeroizing::new(pw))
}

/// Delete the secret for `account_id`. A missing entry is treated as
/// success (idempotent delete).
pub fn delete(account_id: &str) -> Result<(), String> {
    match entry(account_id)?.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keychain delete failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_key_is_namespaced() {
        assert_eq!(account_key("abc123"), "acct:abc123");
    }
}
