// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Finding a channel's MuxBus tokens (`Store::muxbus_load_tokens`): one step per
//! place they can be, tried in this order, the first that has them wins:
//!
//! 1. this platform's current layout (`preferred_layout`),
//! 2. the other platform's layout, migrated in place (`other_layout`),
//! 3. the plaintext SQL columns of a row that predates keychain storage,
//! 4. the host-wide session of the same account, adopted once by a channel that
//!    signed in before tokens were scoped per channel.
//!
//! A keychain READ failure at any step falls back to the plaintext SQL columns
//! when they have data (`read_failed`): a transient storage failure must not
//! present as a full logout while usable data sits right there.
//!
//! `allow_migration` gates only the self-heal WRITES (`false` for
//! `muxbus_is_fresh`'s side-effect-free contract, `true` for a normal load).
//! The caller (`muxbus_load_impl`) already holds the store's save lock and the
//! channel's cross-process lock.

use rusqlite::params;

use super::error::StoreError;
use super::muxbus::{
    blob_key, current_row_id, delete_split_tokens, generation_as_number, keychain_namespace, lock_namespace, read_any_layout, read_split_settled,
    read_split_tokens, same_account, write_single_blob, write_split_tokens, MuxBusBlob, MuxBusTokens, PriorKeychainState, SplitRead,
    GLOBAL_KEYCHAIN_NS,
};
use super::store::Store;
use crate::identity::secret_store;

/// The plaintext SQL columns of a row, which hold tokens only if it predates keychain storage.
struct Legacy<'a> {
    access: &'a str,
    refresh: &'a str,
    id: &'a str,
}

impl Legacy<'_> {
    fn has_tokens(&self) -> bool {
        !self.access.is_empty()
    }

    fn tokens(&self) -> MuxBusTokens {
        MuxBusTokens { access_token: self.access.to_string(), refresh_token: self.refresh.to_string(), id_token: self.id.to_string() }
    }
}

/// What a step found: the tokens, or nothing, so the next step looks.
enum Found {
    Tokens(MuxBusTokens),
    Next,
}

type Step = Result<Found, StoreError>;

/// A keychain read failed: the plaintext columns when they hold the tokens, else the error.
fn read_failed<E: std::fmt::Display>(e: E, as_error: impl FnOnce(E) -> StoreError, legacy: &Legacy) -> Step {
    if legacy.has_tokens() {
        tracing::warn!(error = %e, "muxbus: keychain read failed, falling back to legacy plaintext columns");
        return Ok(Found::Tokens(legacy.tokens()));
    }
    Err(as_error(e))
}

fn keychain_read_error(e: String) -> StoreError {
    StoreError::Other(format!("muxbus: keychain read failed: {e}"))
}

fn corrupted_blob(e: serde_json::Error) -> StoreError {
    StoreError::Other(format!("muxbus: stored keychain blob is corrupted: {e}"))
}

/// This platform's own layout: Windows needs the chunked per-field layout (its
/// 1280-char keychain entry cap); macOS and Linux use the single combined blob
/// (see `blob_key`). A hit needs no migration: it is already the right format.
fn preferred_layout(ns: &str, allow_migration: bool, legacy: &Legacy) -> Step {
    if cfg!(target_os = "windows") {
        windows_split_layout(ns, legacy)
    } else {
        unix_blob_layout(ns, allow_migration, legacy)
    }
}

/// The other platform's layout as a migration source. On Windows that is the
/// pre-2026-08-03 single blob. On macOS and Linux it is the chunked layout an
/// install from before the 2026-08-19 fix may have written (every platform
/// wrote it then); see retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md.
fn other_layout(ns: &str, allow_migration: bool, legacy: &Legacy) -> Step {
    if cfg!(target_os = "windows") {
        windows_legacy_blob(ns, allow_migration, legacy)
    } else {
        unix_legacy_split_layout(ns, allow_migration, legacy)
    }
}

/// Write `tokens` in this platform's preferred layout.
fn write_preferred_layout(ns: &str, tokens: &MuxBusTokens) -> Result<Vec<(String, PriorKeychainState)>, StoreError> {
    if cfg!(target_os = "windows") {
        write_split_tokens(ns, tokens)
    } else {
        write_single_blob(ns, tokens)
    }
}

fn windows_split_layout(ns: &str, legacy: &Legacy) -> Step {
    match read_split_settled(ns) {
        Ok(SplitRead::Complete(tokens, _generation)) => Ok(Found::Tokens(tokens)),
        // Not yet on the split layout, or torn and still torn after
        // the retries: check the other sources.
        Ok(SplitRead::Absent) => Ok(Found::Next),
        // Torn, but its refresh token is whole: hand back just that,
        // and the next refresh (which checks the account) saves a
        // whole set again. No sign-in needed.
        Ok(SplitRead::Torn { refresh: Some(refresh_token) }) => {
            tracing::warn!(
                "muxbus: split keychain entries are torn and stayed so on retry — \
                 keeping the refresh token so the next refresh repairs them"
            );
            Ok(Found::Tokens(MuxBusTokens { refresh_token, ..Default::default() }))
        }
        Ok(SplitRead::Torn { refresh: None }) => {
            tracing::warn!(
                "muxbus: split keychain entries are torn (mismatched generation stamps \
                 or incomplete) and stayed so on retry, with no whole refresh token — \
                 treating as not signed in; signing in again repairs it"
            );
            Ok(Found::Next)
        }
        Err(e) => read_failed(e, |e| e, legacy),
    }
}

fn unix_blob_layout(ns: &str, allow_migration: bool, legacy: &Legacy) -> Step {
    match secret_store::get_optional_if_trusted(&blob_key(ns)) {
        Ok(Some(blob)) => {
            // A blob that doesn't parse means something wrote bad data, not
            // "no credential": an error, not a silent logout.
            let parsed: MuxBusBlob = serde_json::from_str(&blob).map_err(corrupted_blob)?;
            Ok(Found::Tokens(blob_or_fresher_split(ns, allow_migration, parsed)))
        }
        // Not yet on the single-blob layout: check the other sources.
        Ok(None) => Ok(Found::Next),
        Err(e) => read_failed(e, keychain_read_error, legacy),
    }
}

/// The tokens of a blob, unless split entries beside it are fresher.
///
/// A blob and split entries can coexist on this machine two ways, and they
/// don't agree on which is fresher, so an actual timestamp comparison (not a
/// fixed direction) is the only way to get both right (see `MuxBusBlob`):
///   1. An earlier blob-to-split migration (it ran on every platform) wrote
///      split entries but its own best-effort blob delete failed: split is
///      fresher there.
///   2. The self-heal below (split to blob) can leave stale split leftovers if
///      `delete_split_tokens` partially fails; a later `muxbus_save` (macOS and
///      Linux only ever write the blob) then makes the blob fresher.
fn blob_or_fresher_split(ns: &str, allow_migration: bool, parsed: MuxBusBlob) -> MuxBusTokens {
    let blob_generation = generation_as_number(&parsed.generation);
    match read_split_tokens(ns) {
        Ok(Some((split_tokens, split_generation))) => {
            if generation_as_number(&split_generation) > blob_generation {
                // Split is genuinely newer: finish the interrupted migration
                // into the preferred single-blob format.
                if allow_migration {
                    match write_single_blob(ns, &split_tokens) {
                        Ok(_) => delete_split_tokens(ns),
                        Err(_) => {
                            tracing::warn!(
                                "muxbus: keychain write failed reconciling coexisting \
                                 blob + split entries (split was fresher) — leaving \
                                 both in place for now"
                            );
                        }
                    }
                }
                return split_tokens;
            }
            // The blob is the same age or newer, so it is authoritative. The
            // split entries are a stale leftover from an incomplete cleanup:
            // finish it now instead of letting it mislead the next load too.
            if allow_migration {
                delete_split_tokens(ns);
            }
        }
        Ok(None) => {} // no coexistence: the blob alone is authoritative
        Err(e) => {
            // Can't confirm whether split entries coexist: use the blob in
            // hand rather than failing a read that would otherwise succeed.
            tracing::warn!(
                error = %e,
                "muxbus: couldn't check for a coexisting split-entry layout — \
                 using the blob value"
            );
        }
    }
    parsed.tokens
}

fn windows_legacy_blob(ns: &str, allow_migration: bool, legacy: &Legacy) -> Step {
    match secret_store::get_optional(&blob_key(ns)) {
        Ok(Some(blob)) => {
            let tokens: MuxBusTokens = serde_json::from_str(&blob).map_err(corrupted_blob)?;
            if allow_migration {
                match write_split_tokens(ns, &tokens) {
                    Ok(_) => {
                        let _ = secret_store::delete(&blob_key(ns));
                    }
                    Err(_) => {
                        tracing::warn!(
                            "muxbus: keychain write failed migrating the legacy combined-blob \
                             entry to split entries — leaving the old entry in place for now"
                        );
                    }
                }
            }
            Ok(Found::Tokens(tokens))
        }
        Ok(None) => Ok(Found::Next),
        Err(e) => read_failed(e, keychain_read_error, legacy),
    }
}

fn unix_legacy_split_layout(ns: &str, allow_migration: bool, legacy: &Legacy) -> Step {
    match read_split_settled(ns) {
        // Pre-blob split entries left torn: as on Windows, the refresh's save writes a whole blob.
        Ok(SplitRead::Torn { refresh: Some(refresh_token) }) => {
            tracing::warn!(
                "muxbus: legacy split keychain entries are torn — keeping the refresh token \
                 so the next refresh repairs them"
            );
            Ok(Found::Tokens(MuxBusTokens { refresh_token, ..Default::default() }))
        }
        Ok(SplitRead::Torn { refresh: None }) => {
            tracing::warn!(
                "muxbus: legacy split keychain entries are torn with no whole refresh token — \
                 treating as not signed in; signing in again repairs it"
            );
            Ok(Found::Next)
        }
        Ok(SplitRead::Complete(tokens, _generation)) => {
            // No coexisting blob was found (the blob step already checked and
            // returned): self-heal by collapsing the pre-fix chunked entries
            // into one blob, so every later load, and every OS Keychain
            // consent prompt, touches one entry instead of up to twelve.
            if allow_migration {
                match write_single_blob(ns, &tokens) {
                    Ok(_) => delete_split_tokens(ns),
                    Err(_) => {
                        tracing::warn!(
                            "muxbus: keychain write failed migrating chunked entries to a \
                             single blob — leaving the old entries in place for now"
                        );
                    }
                }
            }
            Ok(Found::Tokens(tokens))
        }
        Ok(SplitRead::Absent) => Ok(Found::Next),
        Err(e) if secret_store::is_needs_consent(&e.to_string()) => Ok(Found::Next), // saved by an untrusted older build: signed out
        Err(e) => read_failed(e, |e| e, legacy),
    }
}

/// This channel has its own SQL row (the caller got past the row gate) but
/// nothing under its own keychain namespace: it signed in before tokens were
/// scoped per channel, when they went to the host-wide set. Adopt that set
/// once, read-only — the global entries are never written or deleted from here,
/// so other channels and `stable` keep theirs. From the next save on, this
/// channel's tokens are its own. Only the same account's: `stable` may be
/// signed in as another, and its tokens must not pair with this row.
fn adopt_host_wide_session(ns: &str, user_sub: &str, allow_migration: bool) -> Result<Option<MuxBusTokens>, StoreError> {
    // Its own lock: `stable` saves the host-wide set under that one.
    // Always taken after this channel's, never the other way round.
    let global_tokens = {
        let _global = lock_namespace(GLOBAL_KEYCHAIN_NS)?;
        read_any_layout(GLOBAL_KEYCHAIN_NS)
    };
    let Some(tokens) = global_tokens.filter(|t| same_account(t, user_sub)) else {
        return Ok(None);
    };
    tracing::info!(namespace = %ns, "muxbus: adopting the host-wide session for this channel (one-time, pre-per-channel sign-in)");
    if allow_migration && write_preferred_layout(ns, &tokens).is_err() {
        tracing::warn!(
            "muxbus: couldn't copy the adopted session into this channel's \
             keychain namespace — it stays readable from the host-wide set for now"
        );
    }
    Ok(Some(tokens))
}

impl Store {
    /// Resolve the three token fields from the first place that has them (see the module docs).
    pub(super) fn muxbus_load_tokens(
        &self,
        allow_migration: bool,
        legacy_access: &str,
        legacy_refresh: &str,
        legacy_id: &str,
        user_sub: &str,
    ) -> Result<MuxBusTokens, StoreError> {
        let legacy = Legacy { access: legacy_access, refresh: legacy_refresh, id: legacy_id };
        let ns = keychain_namespace();

        if let Found::Tokens(tokens) = preferred_layout(&ns, allow_migration, &legacy)? {
            return Ok(tokens);
        }
        if let Found::Tokens(tokens) = other_layout(&ns, allow_migration, &legacy)? {
            return Ok(tokens);
        }
        if legacy.has_tokens() {
            return Ok(self.adopt_plaintext_columns(&ns, allow_migration, &legacy));
        }
        if ns != GLOBAL_KEYCHAIN_NS {
            if let Some(tokens) = adopt_host_wide_session(&ns, user_sub, allow_migration)? {
                return Ok(tokens);
            }
        }
        Ok(MuxBusTokens::default())
    }

    /// The row predates keychain-backed storage: use the plaintext columns. With
    /// migration allowed, also self-heal once, by writing them into this
    /// platform's preferred layout and blanking the SQL columns, so every later
    /// load takes the keychain path. Without it (`muxbus_is_fresh`) the read
    /// stays read-only: the same values a migration would have written, without
    /// touching the keychain or the SQL columns.
    fn adopt_plaintext_columns(&self, ns: &str, allow_migration: bool, legacy: &Legacy) -> MuxBusTokens {
        let tokens = legacy.tokens();
        if !allow_migration {
            return tokens;
        }
        match write_preferred_layout(ns, &tokens) {
            Ok(_) => {
                let conn = self.conn.lock().unwrap();
                let _ = conn.execute(
                    "UPDATE db_muxbus_credentials
                     SET access_token = '', refresh_token = '', id_token = ''
                     WHERE id = ?1",
                    params![current_row_id()],
                );
            }
            Err(_) => {
                tracing::warn!(
                    "muxbus: keychain write failed during lazy migration — \
                     leaving plaintext columns in place for now"
                );
            }
        }
        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_keychain_read_uses_the_plaintext_columns_when_they_have_tokens_and_is_an_error_otherwise() {
        let with = Legacy { access: "a", refresh: "r", id: "i" };
        let Ok(Found::Tokens(t)) = read_failed("locked".to_string(), keychain_read_error, &with) else { panic!("the plaintext tokens") };
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str(), t.id_token.as_str()), ("a", "r", "i"));

        let without = Legacy { access: "", refresh: "r-only", id: "" };
        let err = read_failed("locked".to_string(), keychain_read_error, &without).err().expect("an error").to_string();
        assert!(err.contains("muxbus: keychain read failed: locked"), "{err}");
        // The split-entry paths hand their own error back unwrapped.
        let err = read_failed(StoreError::Other("as is".into()), |e| e, &without).err().expect("an error").to_string();
        assert!(err.contains("as is") && !err.contains("keychain read failed"), "{err}");
    }
}
