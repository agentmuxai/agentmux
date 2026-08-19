// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;
use crate::identity::secret_store;

/// `secret_store` is otherwise keyed by identity-account id; MuxBus has
/// exactly one, global credential set, so it uses a fixed sentinel key —
/// shared with the broker's credential id, see `crate::muxbus::CREDENTIAL_ID`.
const MUXBUS_KEYCHAIN_ID: &str = crate::muxbus::CREDENTIAL_ID;

/// Single-entry key holding all three tokens as one JSON blob. Two different
/// roles depending on platform (see `muxbus_load_tokens` / `muxbus_save`):
///
/// - **Windows**: legacy (pre-2026-08-03) only. Windows Credential Manager
///   caps a single entry's blob at 2560 bytes (`CRED_MAX_CREDENTIAL_BLOB_SIZE`)
///   — a Cognito `id_token` alone can approach that, and combined with
///   `access_token`+`refresh_token` reliably exceeds it (see
///   `docs/specs/PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03.md`). Kept
///   only as a one-time migration source there; every Windows write goes
///   through the chunked per-field layout below instead.
/// - **macOS/Linux**: the CURRENT, preferred format. These platforms have no
///   comparable blob-size cap, so the chunked layout's only purpose there
///   was to be platform-uniform with Windows — but each of its ~12 separate
///   keychain entries is its own OS-level access-consent decision, and macOS
///   Keychain prompts per-entry until each is individually trusted. Granting
///   "Always Allow" on one entry just surfaced the next entry's own prompt,
///   making the whole flow look broken (retro-macos-muxbus-keychain-prompt-
///   storm-2026-08-19.md). One combined blob means one prompt, that one
///   "Always Allow" click actually sticks.
const LEGACY_BLOB_KEYCHAIN_ID: &str = MUXBUS_KEYCHAIN_ID;

const FIELD_ACCESS: &str = "access";
const FIELD_REFRESH: &str = "refresh";
const FIELD_ID: &str = "id";

fn field_key(field: &str) -> String {
    format!("{MUXBUS_KEYCHAIN_ID}:{field}")
}

/// A first fix attempt (splitting the combined blob into one keychain entry
/// per field) turned out NOT to be sufficient: live-tested against a real
/// Cognito login, the exact same "Attribute 'password' is longer than
/// platform limit of 2560 chars" error still fired — a single token field
/// can itself exceed the cap depending on the app client's token content
/// (custom claims, refresh token length), not just the three combined. So
/// each field is further chunked into as many entries as it takes, tracked
/// by an explicit `<field>:count` entry (`write_chunked_field` /
/// `read_chunked_field` below) — this holds regardless of any individual
/// token's real-world size, instead of relying on an assumption about it.
///
/// A second live test with a 1800-char budget hit the SAME error again —
/// the `keyring` crate's Windows backend checks
/// `password.encode_utf16().count() * 2 > CRED_MAX_CREDENTIAL_BLOB_SIZE`
/// (2560 *bytes*), but its own error message reports that raw byte count as
/// if it were a char limit ("longer than platform limit of 2560 chars").
/// Since Windows stores the value as UTF-16 (2 bytes/char), the real
/// character budget is `CRED_MAX_CREDENTIAL_BLOB_SIZE / 2` = 1280, not 2560
/// — `keyring-2.3.3/src/windows.rs:182` vs. its own error text at
/// `error.rs:72`. 1800 was 40% over that real limit. Budget well under 1280
/// here, not just under the misleading 2560 the error text implies.
const MAX_CHUNK_LEN: usize = 1000;

fn chunk_key(field_key: &str, index: usize) -> String {
    format!("{field_key}:{index}")
}

fn count_key(field_key: &str) -> String {
    format!("{field_key}:count")
}

fn generation_key(field_key: &str) -> String {
    format!("{field_key}:gen")
}

/// A per-write identifier stamped onto all three fields by a single
/// `write_split_tokens` call, so `read_split_tokens` can detect a torn
/// write ACROSS a process crash — not the concurrent-read-in-one-process
/// case `muxbus_load_impl`'s lock already covers, but a kill signal
/// landing mid-write, between one field's `write_chunked_field` call
/// completing and the next one starting (reagent P2: each field is
/// individually self-consistent, per `write_chunked_field`'s own
/// chunk+rollback atomicity, but with no lock surviving the process
/// there's nothing to stop the OLD stale value for the not-yet-reached
/// field from still being present when the other two ARE the fresh ones —
/// silently reconstructing a token set that pairs pieces from two
/// different login sessions). High-resolution wall-clock nanos is unique
/// enough for this purpose in practice; a formal UUID would work too but
/// adds a dependency for no real benefit here.
fn new_generation() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

/// Pure chunking split — no keychain I/O — so the boundary math is testable
/// without touching the real OS keychain. A token's bytes are always ASCII
/// in practice (JWTs / opaque base64url strings), so splitting on byte
/// boundaries never lands mid-character; `unwrap_or("")` is a defensive
/// fallback only, not an expected path.
fn chunk_value(value: &str) -> Vec<&str> {
    if value.is_empty() {
        return vec![""];
    }
    value
        .as_bytes()
        .chunks(MAX_CHUNK_LEN)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect()
}

/// A previous, longer value at a field could in principle need many chunks;
/// no real Cognito token comes remotely close (`MAX_PLAUSIBLE_CHUNKS *
/// MAX_CHUNK_LEN` = 32,000 chars). Used only as `muxbus_clear`'s fallback
/// deletion bound when the real count can't be read (see its call site) —
/// `secret_store::delete` on a non-existent entry is a no-op success, so
/// scanning past the real count there is harmless, just wasted calls.
const MAX_PLAUSIBLE_CHUNKS: usize = 32;

/// `Ok(0)` means the `:count` entry genuinely doesn't exist (field never
/// written under the chunked layout). `Err` means the read itself failed —
/// reagent P1: a prior version of this collapsed both into the same `0`,
/// which `muxbus_clear` used to bound its per-field deletion loop
/// (`for i in 0..count`) — a transient read failure made that loop delete
/// NOTHING, while the field's `:count` key was still deleted unconditionally
/// right after, so logout appeared to succeed while the real token chunks
/// were silently orphaned in the OS keychain. Callers must not treat `Err`
/// as "0 chunks."
fn read_chunk_count(field_key: &str) -> Result<usize, StoreError> {
    match secret_store::get_optional(&count_key(field_key)) {
        Ok(Some(v)) => v
            .parse::<usize>()
            .map_err(|e| StoreError::Other(format!("muxbus: corrupted chunk count for {field_key}: {e}"))),
        Ok(None) => Ok(0),
        Err(e) => Err(StoreError::Other(format!("muxbus: keychain read failed: {e}"))),
    }
}

#[derive(Debug, Clone, Default)]
pub struct MuxBusCredentials {
    pub cognito_domain: String,
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub expires_at: i64,
    pub user_email: String,
    pub user_sub: String,
}

/// The actual secret material. Every field is written to its own keychain
/// entry (`field_key`); this struct only groups them for callers and for
/// deserializing the legacy combined-blob format during migration.
/// Everything else on `MuxBusCredentials` is non-secret metadata and stays
/// in SQLite, matching how `SecretRef::Keychain` accounts already split
/// pointer-metadata (DB) from plaintext (keychain) elsewhere in this codebase.
#[derive(Serialize, Deserialize, Default)]
struct MuxBusTokens {
    access_token: String,
    refresh_token: String,
    id_token: String,
}

/// What a keychain entry held immediately before a write to it, so a later
/// failure (the SQL write in `muxbus_save`, or a sibling field's write in
/// `write_split_tokens`) can be rolled back to that exact prior state.
/// Three-way, not a bool: reagent P1 on #2260 caught a first version of this
/// that collapsed `Ok(None)` ("genuinely no prior entry") and `Err` ("the
/// read itself failed, prior state UNKNOWN") into the same `None`, so an
/// unrelated transient read failure would make the rollback branch delete a
/// real, valid, previously-stored credential it simply couldn't read —
/// turning a transient glitch into a forced full re-login. `Unknown` gets
/// neither restore nor delete: we truly don't know what was there, so the
/// only safe move is to leave whatever the write just wrote and log loudly.
enum PriorKeychainState {
    Existed(zeroize::Zeroizing<String>),
    Absent,
    Unknown,
}

impl PriorKeychainState {
    fn capture(key: &str) -> Self {
        match secret_store::get_optional(key) {
            Ok(Some(v)) => PriorKeychainState::Existed(v),
            Ok(None) => PriorKeychainState::Absent,
            Err(_) => PriorKeychainState::Unknown,
        }
    }

    fn restore(&self, key: &str) -> Result<(), String> {
        match self {
            PriorKeychainState::Existed(old) => secret_store::put(key, old.as_str()),
            PriorKeychainState::Absent => secret_store::delete(key),
            PriorKeychainState::Unknown => Ok(()),
        }
    }
}

/// Write `value` to `field_key` as one or more chunk entries (each under
/// `MAX_CHUNK_LEN`) plus a `:count` entry, clearing any stale trailing
/// chunks left over from a previous, longer value at this key, and stamps
/// `generation` (same value across all three fields for one
/// `write_split_tokens` call — see `new_generation`'s doc comment) on this
/// field's `:gen` entry. Rolls back everything this call touched if any
/// single write/delete fails partway through.
///
/// Write order matters here beyond just "rollback on failure" (reagent P1:
/// a prior version wrote chunks, then cleared stale trailing chunks, then
/// updated `:count`, then `:gen`, in that order — a process CRASH between
/// the chunk-write and the trailing-chunk-delete left `:count` still
/// pointing at the OLD larger count while the leading chunks already held
/// NEW content; on restart `read_chunked_field` would read exactly
/// `old_count` chunks — none technically missing, so no error — silently
/// reconstructing a spliced new-prefix/old-suffix value that's neither the
/// old nor the new token, with `:gen` untouched so `read_split_tokens`'s
/// cross-field generation check never even saw a mismatch to catch). This
/// version deletes `:gen` FIRST, before touching any chunk content, and
/// writes the fresh `generation` value LAST, only once chunks + `:count`
/// are fully consistent — `read_chunked_field` already treats a missing
/// `:gen` on an otherwise-populated field as an error, so a crash ANYWHERE
/// in between now makes this field unambiguously detectable as torn,
/// rather than silently reconstructible.
///
/// On success, returns the PRE-call state of every entry touched (`:gen`,
/// chunks, `:count`), so a caller doing more work of its own afterward
/// (`write_split_tokens` covering a sibling field, or `muxbus_save`'s SQL
/// write) can roll all of it back together if that later step fails too.
fn write_chunked_field(
    field_key: &str,
    value: &str,
    generation: &str,
) -> Result<Vec<(String, PriorKeychainState)>, StoreError> {
    let new_chunks = chunk_value(value);
    let new_count = new_chunks.len();
    // Propagate a genuine read failure rather than assuming 0 — silently
    // treating "couldn't read the old count" as "there were no stale
    // trailing chunks" would skip cleaning up real leftover chunk data from
    // a previous, longer value (same class of bug as the read_chunk_count
    // doc comment describes for muxbus_clear).
    let old_count = read_chunk_count(field_key)?;

    let mut priors: Vec<(String, PriorKeychainState)> = Vec::with_capacity(new_count.max(old_count) + 2);
    let rollback = |priors: &[(String, PriorKeychainState)]| {
        for (key, prior) in priors {
            if let Err(re) = prior.restore(key) {
                tracing::warn!(
                    error = %re,
                    key = %key,
                    "muxbus: rollback after a partial chunked-field write failure also failed \
                     for this entry — keychain may now hold a stale value for it"
                );
            }
        }
    };

    // Invalidate this field before touching any chunk content — see the
    // doc comment above for why this specific ordering is load-bearing.
    // The captured `gen_prior` (this field's TRUE pre-call `:gen` state) is
    // what a later rollback restores to, whether the failure happens here,
    // mid-chunk-write, or in the final `:gen` write at the bottom of this
    // function — there is deliberately only ONE priors entry for this key.
    let gk = generation_key(field_key);
    let gen_prior = PriorKeychainState::capture(&gk);
    if let Err(e) = secret_store::delete(&gk) {
        return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
    }
    priors.push((gk.clone(), gen_prior));

    for (i, chunk) in new_chunks.iter().enumerate() {
        let key = chunk_key(field_key, i);
        let prior = PriorKeychainState::capture(&key);
        if let Err(e) = secret_store::put(&key, chunk) {
            rollback(&priors);
            return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
        }
        priors.push((key, prior));
    }

    // A previous, longer value left trailing chunks beyond the new count —
    // clear them, or a future read would append stale old data past the
    // new count boundary... except reads stop at `count`, so leftover
    // chunks are actually inert. Clear them anyway: cheap, and avoids ever
    // depending on that "reads ignore trailing chunks" invariant holding.
    for i in new_count..old_count {
        let key = chunk_key(field_key, i);
        let prior = PriorKeychainState::capture(&key);
        if let Err(e) = secret_store::delete(&key) {
            rollback(&priors);
            return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
        }
        priors.push((key, prior));
    }

    let ck = count_key(field_key);
    let prior = PriorKeychainState::capture(&ck);
    if let Err(e) = secret_store::put(&ck, &new_count.to_string()) {
        rollback(&priors);
        return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
    }
    priors.push((ck, prior));

    // Write the fresh generation LAST — only once chunks + :count are fully
    // consistent. This is what makes the field valid (readable) again; the
    // `gk` rollback entry already pushed above (this field's true pre-call
    // state) is reused if THIS write itself fails, not a fresh capture —
    // capturing now would wrongly record "Absent" (we deleted it ourselves
    // above) as what to roll back to.
    if let Err(e) = secret_store::put(&gk, generation) {
        rollback(&priors);
        return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
    }

    Ok(priors)
}

/// Read `field_key`'s value + generation stamp back from its chunk entries.
/// `Ok(None)` means no `:count` entry exists yet (this field was never
/// written under the chunked layout — caller falls through to the
/// legacy-blob / legacy-plaintext sources). `Err` means a read genuinely
/// failed, or the chunk state is internally inconsistent (a chunk went
/// missing between the count write and now) — not just "no entry". A
/// missing `:gen` entry on an otherwise-complete field is treated as a read
/// failure too, not defaulted to some sentinel — see `read_split_tokens`,
/// which needs a REAL generation to compare across fields, not a value
/// that would spuriously "match" another field's own missing generation.
fn read_chunked_field(field_key: &str) -> Result<Option<(String, String)>, StoreError> {
    let count = match secret_store::get_optional(&count_key(field_key)) {
        Ok(Some(v)) => v.parse::<usize>().map_err(|e| {
            StoreError::Other(format!("muxbus: corrupted chunk count for {field_key}: {e}"))
        })?,
        Ok(None) => return Ok(None),
        Err(e) => return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}"))),
    };
    let mut value = String::new();
    for i in 0..count {
        match secret_store::get_optional(&chunk_key(field_key, i)) {
            Ok(Some(chunk)) => value.push_str(&chunk),
            Ok(None) => {
                return Err(StoreError::Other(format!(
                    "muxbus: missing chunk {i}/{count} for {field_key} — keychain state is inconsistent"
                )));
            }
            Err(e) => return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}"))),
        }
    }
    let generation = match secret_store::get_optional(&generation_key(field_key)) {
        Ok(Some(g)) => g.to_string(),
        Ok(None) => {
            return Err(StoreError::Other(format!(
                "muxbus: missing generation stamp for {field_key} — keychain state is inconsistent"
            )));
        }
        Err(e) => return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}"))),
    };
    Ok(Some((value, generation)))
}

/// Write all three tokens as one combined JSON blob under a single keychain
/// entry (`LEGACY_BLOB_KEYCHAIN_ID` — the CURRENT preferred format on
/// macOS/Linux, see that constant's doc comment). One `SecItemAdd`/
/// `SecItemUpdate` call is inherently atomic at the OS level, so this needs
/// none of `write_split_tokens`'s cross-field generation-stamp/rollback
/// machinery — there's only one field. Returns the pre-call state wrapped in
/// the same `Vec<(String, PriorKeychainState)>` shape `write_split_tokens`
/// returns, so `muxbus_save`'s SQL-failure rollback branch works unchanged
/// regardless of which one ran.
fn write_single_blob(tokens: &MuxBusTokens) -> Result<Vec<(String, PriorKeychainState)>, StoreError> {
    let prior = PriorKeychainState::capture(LEGACY_BLOB_KEYCHAIN_ID);
    let blob = serde_json::to_string(tokens)
        .map_err(|e| StoreError::Other(format!("muxbus: failed to serialize tokens: {e}")))?;
    if let Err(e) = secret_store::put(LEGACY_BLOB_KEYCHAIN_ID, &blob) {
        return Err(StoreError::Other(format!("muxbus: keychain write failed: {e}")));
    }
    Ok(vec![(LEGACY_BLOB_KEYCHAIN_ID.to_string(), prior)])
}

/// Delete every entry the chunked per-field layout could have written
/// (`FIELD_ACCESS`/`FIELD_REFRESH`/`FIELD_ID`'s chunks + `:count` + `:gen`).
/// Shared by `muxbus_clear` (unconditional logout cleanup, any platform) and
/// the macOS/Linux migration path in `muxbus_load_tokens` (collapsing a
/// pre-fix install's chunked entries into the single-blob format — see
/// `LEGACY_BLOB_KEYCHAIN_ID`'s doc comment). Best-effort: `secret_store::delete`
/// on a non-existent entry is a no-op success, and a real delete failure here
/// just leaves an orphaned, unreadable-without-the-others chunk behind rather
/// than losing anything live.
fn delete_split_tokens() {
    for field in [FIELD_ACCESS, FIELD_REFRESH, FIELD_ID] {
        let fk = field_key(field);
        // A count-read failure must NOT be treated as "0 chunks" (see
        // read_chunk_count's doc comment) — that would delete nothing here
        // while still unconditionally deleting the `:count` key below,
        // orphaning the real token chunks in the OS keychain. Fall back to
        // scanning a generous bound instead: `secret_store::delete` on a
        // non-existent entry is a no-op success, so deleting past the real
        // count is harmless — it guarantees actual cleanup even when the
        // count itself is unreadable.
        let count = match read_chunk_count(&fk) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    field = %fk,
                    "muxbus: couldn't read this field's chunk count while deleting split \
                     entries — falling back to a bounded scan so real token chunks still get \
                     deleted"
                );
                MAX_PLAUSIBLE_CHUNKS
            }
        };
        for i in 0..count {
            let _ = secret_store::delete(&chunk_key(&fk, i));
        }
        let _ = secret_store::delete(&count_key(&fk));
        let _ = secret_store::delete(&generation_key(&fk));
    }
}

/// Write all three token fields, each independently chunked
/// (`write_chunked_field`). On a later field's failure, rolls back every
/// entry every earlier field in this call already wrote — otherwise a
/// partial failure would leave the three fields holding a mix of old and
/// new tokens, which is worse than either the old or the new set alone.
///
/// On success, returns every entry's PRE-call state (across all three
/// fields) so `muxbus_save`'s SQL-write failure branch can roll all of it
/// back together too.
fn write_split_tokens(tokens: &MuxBusTokens) -> Result<Vec<(String, PriorKeychainState)>, StoreError> {
    let generation = new_generation();
    let fields: [(String, &str); 3] = [
        (field_key(FIELD_ACCESS), tokens.access_token.as_str()),
        (field_key(FIELD_REFRESH), tokens.refresh_token.as_str()),
        (field_key(FIELD_ID), tokens.id_token.as_str()),
    ];
    let mut all_priors: Vec<(String, PriorKeychainState)> = Vec::new();
    for (key, value) in fields {
        match write_chunked_field(&key, value, &generation) {
            Ok(priors) => all_priors.extend(priors),
            Err(e) => {
                for (done_key, done_prior) in &all_priors {
                    if let Err(re) = done_prior.restore(done_key) {
                        tracing::warn!(
                            error = %re,
                            key = %done_key,
                            "muxbus: rollback after a partial split-token write failure also failed \
                             for this entry — keychain may now hold a stale value for it"
                        );
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(all_priors)
}

/// Read the three split-entry (chunked) tokens. `Ok(None)` means none of the
/// three fields exist yet (not migrated to this layout — caller falls
/// through to the legacy-blob / legacy-plaintext sources). `Err` means a
/// read itself failed (locked keychain, no Secret Service daemon,
/// permission denied — not just "no entry"), which the caller may still
/// recover from via a legacy fallback.
fn read_split_tokens() -> Result<Option<MuxBusTokens>, StoreError> {
    let access = read_chunked_field(&field_key(FIELD_ACCESS))?;
    let refresh = read_chunked_field(&field_key(FIELD_REFRESH))?;
    let id = read_chunked_field(&field_key(FIELD_ID))?;

    match (access, refresh, id) {
        (None, None, None) => Ok(None),
        (Some((access_token, gen_a)), Some((refresh_token, gen_r)), Some((id_token, gen_i))) => {
            // reagent P2: write_split_tokens's own writes are individually
            // consistent per-field (write_chunked_field's chunk+rollback
            // atomicity) but not atomic ACROSS fields — a process kill
            // between two fields' writes can leave one field's stale OLD
            // value sitting next to the other two fields' fresh values,
            // each independently well-formed, silently pairing tokens from
            // two different login sessions. All three fields' generation
            // stamps only match when they came from the SAME
            // write_split_tokens call — a mismatch means a torn write.
            if gen_a == gen_r && gen_r == gen_i {
                Ok(Some(MuxBusTokens {
                    access_token,
                    refresh_token,
                    id_token,
                }))
            } else {
                tracing::warn!(
                    "muxbus: split keychain entries have mismatched generation stamps (a torn \
                     write from an interrupted save) — treating as not-yet-migrated"
                );
                Ok(None)
            }
        }
        _ => {
            // Shouldn't happen in normal operation — write_split_tokens
            // writes/rolls-back all three fields together — but could
            // follow an interrupted write from a prior crash. Treat as "not
            // yet on the split layout" so the caller falls through to the
            // legacy-blob / legacy-plaintext migration paths rather than
            // silently serving a token set with missing fields.
            tracing::warn!(
                "muxbus: inconsistent split keychain entries (some fields present, some absent) — \
                 treating as not-yet-migrated"
            );
            Ok(None)
        }
    }
}

impl MuxBusCredentials {
    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    pub fn is_valid(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at > Self::now_secs()
    }

    pub fn nearly_expired(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at - Self::now_secs() < 300
    }
}

impl Store {
    pub fn muxbus_load(&self) -> Result<Option<MuxBusCredentials>, StoreError> {
        self.muxbus_load_impl(true)
    }

    /// Side-effect-free (no keychain/SQL WRITE, see `allow_migration` below)
    /// freshness check: valid, non-empty access token that isn't nearing
    /// expiry. Safe for `RefreshScheduler::register`'s `is_fresh` closure,
    /// called as it is under the per-id lock on every `ensure_fresh`/sweep
    /// tick — the concern that closure must avoid (reagent P2 on #2260) is
    /// an unexpected WRITE happening inside a nominally read-only check,
    /// not lock acquisition itself; `muxbus_load_impl` below still takes
    /// `muxbus_save_lock` (reagent P1 on this PR — see its comment) purely
    /// to serialize this read against a concurrent writer, with no side
    /// effect of its own performed while holding it. Any error (including a
    /// genuinely corrupted keychain entry) is treated as "not fresh" rather
    /// than propagated — a freshness *check* has no error channel to
    /// propagate through, and "not fresh" just means the broker will
    /// attempt a refresh, which surfaces the real problem through THAT path
    /// instead.
    pub fn muxbus_is_fresh(&self) -> bool {
        self.muxbus_load_impl(false)
            .ok()
            .flatten()
            .map(|c| !c.access_token.is_empty() && c.is_valid() && !c.nearly_expired())
            .unwrap_or(false)
    }

    /// `allow_migration` gates only the lazy-migration self-heal WRITES in
    /// `muxbus_load_tokens` below (`false` for `muxbus_is_fresh`'s
    /// side-effect-free contract, `true` for the normal `muxbus_load`
    /// path) — it does NOT gate the lock below, which every call takes.
    fn muxbus_load_impl(&self, allow_migration: bool) -> Result<Option<MuxBusCredentials>, StoreError> {
        // reagent P1 on #2260 (write-vs-write race) + reagent P1 on this PR
        // (torn read): held for the ENTIRE keychain-read sequence, not just
        // when `allow_migration` is true. Two independent reasons this must
        // be unconditional now:
        //  1. (#2260) The migration branches in `muxbus_load_tokens` read
        //     the keychain, then (on a cache miss) write fresh tokens to it
        //     and update SQL — without this lock, a concurrent `muxbus_save`
        //     could commit a real refresh between this thread's stale SQL
        //     read and its own keychain write, so the migration then
        //     overwrites the freshly-refreshed keychain entries with old
        //     pre-migration data.
        //  2. (this PR) Each field's tokens now live across MULTIPLE
        //     keychain entries (`write_chunked_field`'s chunks + `:count`),
        //     written sequentially with no atomicity across them — unlike
        //     the single combined-blob write this replaced, which the OS
        //     keychain wrote atomically. An UNLOCKED concurrent read
        //     landing mid-write (previously safe when `allow_migration` was
        //     `false`, i.e. every `muxbus_is_fresh` call) can now read a mix
        //     of old and new chunks for a multi-chunk token and silently
        //     reconstruct a CORRUPTED string — every individual
        //     `get_optional` call still succeeds, so nothing errors, it's
        //     just wrong data. Locking here doesn't reintroduce a write
        //     side effect (see `muxbus_is_fresh`'s own doc comment above);
        //     it only serializes this read against `muxbus_save`/
        //     `muxbus_clear`'s writes, which already take the same lock.
        let _migration_guard = self.muxbus_save_lock.lock().unwrap();
        let row = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT cognito_domain, client_id, access_token, refresh_token, id_token,
                        expires_at, user_email, user_sub
                 FROM db_muxbus_credentials WHERE id = 'global'",
            )?;
            match stmt.query_row([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, // legacy plaintext access_token (pre-keychain rows)
                    row.get::<_, String>(3)?, // legacy plaintext refresh_token
                    row.get::<_, String>(4)?, // legacy plaintext id_token
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }) {
                Ok(r) => r,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        };
        let (cognito_domain, client_id, legacy_access, legacy_refresh, legacy_id, expires_at, user_email, user_sub) = row;

        let tokens = self.muxbus_load_tokens(allow_migration, &legacy_access, &legacy_refresh, &legacy_id)?;

        Ok(Some(MuxBusCredentials {
            cognito_domain,
            client_id,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            id_token: tokens.id_token,
            expires_at,
            user_email,
            user_sub,
        }))
    }

    /// Resolve the three token fields, checking sources in order:
    /// 1. the current split-entry keychain layout (fast path),
    /// 2. the legacy (pre-2026-08-03) single combined-blob keychain entry —
    ///    migrated in place to the split layout when found,
    /// 3. legacy plaintext SQL columns from rows that predate keychain
    ///    storage entirely — migrated in place to the split layout too.
    /// A keychain READ failure at any step falls back to the legacy
    /// plaintext SQL columns when they have data, same as the pre-split-entry
    /// code did — a transient storage failure must not present as a full
    /// logout when we still have usable data sitting right there.
    fn muxbus_load_tokens(
        &self,
        allow_migration: bool,
        legacy_access: &str,
        legacy_refresh: &str,
        legacy_id: &str,
    ) -> Result<MuxBusTokens, StoreError> {
        let legacy_plaintext = || MuxBusTokens {
            access_token: legacy_access.to_string(),
            refresh_token: legacy_refresh.to_string(),
            id_token: legacy_id.to_string(),
        };

        // Check THIS platform's preferred, current layout first — Windows
        // needs the chunked per-field layout (its 1280-char keychain entry
        // cap); macOS/Linux use the single combined blob instead (see
        // LEGACY_BLOB_KEYCHAIN_ID's doc comment for why they differ). No
        // migration needed on a hit here — it's already the right format.
        if cfg!(target_os = "windows") {
            match read_split_tokens() {
                Ok(Some(tokens)) => return Ok(tokens),
                Ok(None) => {} // not yet on the split layout — check other sources below
                Err(e) => {
                    if !legacy_access.is_empty() {
                        tracing::warn!(
                            error = %e,
                            "muxbus: keychain read failed, falling back to legacy plaintext columns"
                        );
                        return Ok(legacy_plaintext());
                    }
                    return Err(e);
                }
            }
        } else {
            match secret_store::get_optional(LEGACY_BLOB_KEYCHAIN_ID) {
                Ok(Some(blob)) => {
                    // reagent P2: a corrupted/unparseable keychain blob used
                    // to silently collapse to MuxBusTokens::default() via
                    // unwrap_or_default() — presenting as a full logout
                    // instead of surfacing that something is actually
                    // corrupted. A malformed blob here means something wrote
                    // bad data, not "no credential" — treat it the same way
                    // a real read error is treated: propagate it.
                    let tokens: MuxBusTokens = serde_json::from_str(&blob).map_err(|e| {
                        StoreError::Other(format!("muxbus: stored keychain blob is corrupted: {e}"))
                    })?;
                    return Ok(tokens);
                }
                Ok(None) => {} // not yet on the single-blob layout — check other sources below
                Err(e) => {
                    if !legacy_access.is_empty() {
                        tracing::warn!(
                            error = %e,
                            "muxbus: keychain read failed, falling back to legacy plaintext columns"
                        );
                        return Ok(legacy_plaintext());
                    }
                    return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}")));
                }
            }
        }

        // Not on this platform's preferred layout — check the OTHER layout
        // as a migration source. On Windows this is the pre-2026-08-03
        // single-blob format. On macOS/Linux this is the chunked layout a
        // PRE-2026-08-19-FIX install on this same platform may have written
        // (every platform wrote it uniformly before this fix existed) — see
        // retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md.
        if cfg!(target_os = "windows") {
            match secret_store::get_optional(LEGACY_BLOB_KEYCHAIN_ID) {
                Ok(Some(blob)) => {
                    let tokens: MuxBusTokens = serde_json::from_str(&blob).map_err(|e| {
                        StoreError::Other(format!("muxbus: stored keychain blob is corrupted: {e}"))
                    })?;
                    if allow_migration {
                        match write_split_tokens(&tokens) {
                            Ok(_) => {
                                let _ = secret_store::delete(LEGACY_BLOB_KEYCHAIN_ID);
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "muxbus: keychain write failed migrating the legacy combined-blob \
                                     entry to split entries — leaving the old entry in place for now"
                                );
                            }
                        }
                    }
                    return Ok(tokens);
                }
                Ok(None) => {}
                Err(e) => {
                    if !legacy_access.is_empty() {
                        tracing::warn!(
                            error = %e,
                            "muxbus: keychain read failed, falling back to legacy plaintext columns"
                        );
                        return Ok(legacy_plaintext());
                    }
                    return Err(StoreError::Other(format!("muxbus: keychain read failed: {e}")));
                }
            }
        } else {
            match read_split_tokens() {
                Ok(Some(tokens)) => {
                    // Self-heal: collapse the pre-fix chunked entries into
                    // one blob so every subsequent load — and every future
                    // OS Keychain consent prompt — touches exactly one
                    // entry instead of up to twelve.
                    if allow_migration {
                        match write_single_blob(&tokens) {
                            Ok(_) => delete_split_tokens(),
                            Err(_) => {
                                tracing::warn!(
                                    "muxbus: keychain write failed migrating chunked entries to a \
                                     single blob — leaving the old entries in place for now"
                                );
                            }
                        }
                    }
                    return Ok(tokens);
                }
                Ok(None) => {}
                Err(e) => {
                    if !legacy_access.is_empty() {
                        tracing::warn!(
                            error = %e,
                            "muxbus: keychain read failed, falling back to legacy plaintext columns"
                        );
                        return Ok(legacy_plaintext());
                    }
                    return Err(e);
                }
            }
        }

        // Migration disallowed (muxbus_is_fresh's side-effect-free contract)
        // but legacy plaintext columns still have the data — read-only, same
        // values a migration would have written, just without touching the
        // keychain or SQL columns to get there.
        if !legacy_access.is_empty() {
            let tokens = legacy_plaintext();
            if allow_migration {
                // Lazy migration: this row predates keychain-backed storage.
                // Use the plaintext columns this one time, and self-heal by
                // writing them into this platform's preferred keychain
                // layout + blanking the SQL columns so every subsequent
                // load hits the keychain path instead.
                let write_result = if cfg!(target_os = "windows") {
                    write_split_tokens(&tokens)
                } else {
                    write_single_blob(&tokens)
                };
                match write_result {
                    Ok(_) => {
                        let conn = self.conn.lock().unwrap();
                        let _ = conn.execute(
                            "UPDATE db_muxbus_credentials
                             SET access_token = '', refresh_token = '', id_token = ''
                             WHERE id = 'global'",
                            [],
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            "muxbus: keychain write failed during lazy migration — \
                             leaving plaintext columns in place for now"
                        );
                    }
                }
            }
            return Ok(tokens);
        }

        Ok(MuxBusTokens::default())
    }

    pub fn muxbus_save(&self, creds: &MuxBusCredentials) -> Result<(), StoreError> {
        // Held for the ENTIRE keychain-read/write + SQL-write + rollback
        // sequence below, not just the SQL portion `self.conn`'s own lock
        // covers — reagent P1 on #2260: `muxbus.login` and the broker's
        // refresh closure can each call muxbus_save independently, and
        // without this a race between two concurrent calls could make one
        // call's rollback restore a stale snapshot over the OTHER call's
        // already-committed credential.
        let _save_guard = self.muxbus_save_lock.lock().unwrap();

        // Read the outgoing account's user_sub now, before it's overwritten
        // below, so a genuine account switch (vs. a same-account token
        // refresh) can be detected after the write succeeds and the stale
        // per-agent credential cache cleared. reagentx P0 on PR #2342.
        let previous_user_sub: Option<String> = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT user_sub FROM db_muxbus_credentials WHERE id = 'global'",
                [],
                |row| row.get(0),
            )
            .ok()
        };

        let tokens = MuxBusTokens {
            access_token: creds.access_token.clone(),
            refresh_token: creds.refresh_token.clone(),
            id_token: creds.id_token.clone(),
        };

        // Windows needs the chunked per-field layout (its 1280-char keychain
        // entry cap — see LEGACY_BLOB_KEYCHAIN_ID's doc comment); macOS/Linux
        // have no such cap, so they write the single combined blob instead,
        // to avoid the ~12-separate-consent-prompts problem chunking causes
        // on macOS Keychain specifically. Either function returns each
        // entry's pre-call state so the SQL failure branch below can roll it
        // all back together too if the SQL write itself then fails (reagent
        // P2 on #2260: without this, a keychain write that succeeds followed
        // by a SQL write that then fails — e.g. a transient lock — leaves
        // the FRESH tokens paired with the OLD SQL metadata, a mismatch that
        // previously only self-healed on the next successful save).
        let priors = if cfg!(target_os = "windows") {
            write_split_tokens(&tokens)?
        } else {
            write_single_blob(&tokens)?
        };

        let sql_result = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO db_muxbus_credentials
                     (id, cognito_domain, client_id, access_token, refresh_token, id_token,
                      expires_at, user_email, user_sub)
                 VALUES ('global', ?1, ?2, '', '', '', ?3, ?4, ?5)",
                params![
                    creds.cognito_domain,
                    creds.client_id,
                    creds.expires_at,
                    creds.user_email,
                    creds.user_sub,
                ],
            )
        };
        if let Err(e) = sql_result {
            for (key, prior) in &priors {
                if let Err(re) = prior.restore(key) {
                    tracing::warn!(
                        error = %re,
                        sql_error = %e,
                        key = %key,
                        "muxbus_save: SQL write failed AND restoring this field's prior keychain \
                         state also failed — keychain may now hold a fresh token for it with no \
                         matching SQL metadata until the next successful save"
                    );
                }
            }
            return Err(e.into());
        }

        // Account switch (not a same-account token refresh): the previous
        // account's per-agent M2M credentials must not silently keep
        // authenticating this different account's requests. A first-ever
        // login (previous_user_sub is None/empty) has nothing to clear.
        // reagentx P0 on PR #2342.
        if let Some(prev) = previous_user_sub {
            if !prev.is_empty() && prev != creds.user_sub {
                if let Err(e) = self.agent_credentials_clear_all() {
                    tracing::warn!(
                        error = %e,
                        "muxbus_save: failed to clear stale per-agent credentials after account switch",
                    );
                }
            }
        }

        Ok(())
    }

    pub fn muxbus_clear(&self) -> Result<(), StoreError> {
        // reagent P1 on #2260: without this lock, a concurrent muxbus_save
        // (broker refresh or muxbus.login) can commit its keychain + SQL
        // write after this function's own delete runs, silently
        // resurrecting a credential right after the user disconnected —
        // same race class muxbus_save/muxbus_load_impl already serialize
        // against each other for.
        let _clear_guard = self.muxbus_save_lock.lock().unwrap();
        // Best-effort — a missing/inaccessible keychain entry must not block
        // clearing the (still-useful) SQL row. Clears every chunk + count/gen
        // entry the Windows-only chunked layout could have written (harmless
        // no-ops on macOS/Linux, which never write it, but a pre-fix install
        // there may still have some left over — see `delete_split_tokens`),
        // plus the single combined-blob key every platform's CURRENT write
        // path (`write_single_blob` on macOS/Linux) or legacy migration
        // source (Windows) uses.
        delete_split_tokens();
        let _ = secret_store::delete(LEGACY_BLOB_KEYCHAIN_ID);
        {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM db_muxbus_credentials WHERE id = 'global'", [])?;
        }
        // Logging out invalidates any per-agent credentials provisioned
        // under this account too. reagentx P0 on PR #2342.
        if let Err(e) = self.agent_credentials_clear_all() {
            tracing::warn!(error = %e, "muxbus_clear: failed to clear per-agent credentials");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_json_round_trips() {
        // Legacy combined-blob shape — still needed to deserialize an old
        // single-entry keychain value during one-time migration.
        let tokens = MuxBusTokens {
            access_token: "at".to_string(),
            refresh_token: "rt".to_string(),
            id_token: "it".to_string(),
        };
        let blob = serde_json::to_string(&tokens).unwrap();
        let back: MuxBusTokens = serde_json::from_str(&blob).unwrap();
        assert_eq!(back.access_token, "at");
        assert_eq!(back.refresh_token, "rt");
        assert_eq!(back.id_token, "it");
    }

    #[test]
    fn field_key_is_namespaced_and_distinct_per_field() {
        let access = field_key(FIELD_ACCESS);
        let refresh = field_key(FIELD_REFRESH);
        let id = field_key(FIELD_ID);
        assert_eq!(access, "muxbus:global:access");
        assert_eq!(refresh, "muxbus:global:refresh");
        assert_eq!(id, "muxbus:global:id");
        assert_ne!(access, LEGACY_BLOB_KEYCHAIN_ID);
    }

    /// Regression guard for every bug this file has fixed in sequence: the
    /// original combined-blob-exceeds-2560-bytes bug, the follow-up found
    /// live-testing the first fix (a single field's own token can ALSO
    /// exceed the cap), and the real character limit being HALF the
    /// byte constant the `keyring` crate's error message reports (Windows
    /// stores the value as UTF-16 — see PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03.md
    /// §6-§7). Doesn't exercise the real OS keychain (not available in CI);
    /// asserts the pure chunking math instead: every chunk of an oversized
    /// value stays under the REAL char limit (not the misleading one from
    /// the error text), and concatenating them reconstructs the original
    /// exactly.
    #[test]
    fn chunk_value_keeps_every_chunk_under_windows_credential_blob_limit_and_reconstructs() {
        // `keyring` checks `password.encode_utf16().count() * 2 >
        // CRED_MAX_CREDENTIAL_BLOB_SIZE` (2560 bytes) — so the real char
        // budget for an all-ASCII (1 UTF-16 unit each) token is half that.
        const WINDOWS_CRED_BLOB_BYTE_LIMIT: usize = 2560;
        const WINDOWS_CRED_CHAR_LIMIT: usize = WINDOWS_CRED_BLOB_BYTE_LIMIT / 2;

        // Larger than even the old *combined* blob would have been —
        // proves this doesn't just move the limit, it removes it.
        let oversized_token = "i".repeat(6000);

        let chunks = chunk_value(&oversized_token);
        assert!(chunks.len() > 1, "a value this large must actually split");
        for chunk in &chunks {
            assert!(chunk.len() < WINDOWS_CRED_CHAR_LIMIT);
        }
        assert_eq!(chunks.concat(), oversized_token);
    }

    #[test]
    fn chunk_value_small_input_is_a_single_chunk() {
        assert_eq!(chunk_value("short-token"), vec!["short-token"]);
    }

    #[test]
    fn chunk_value_empty_string_is_one_empty_chunk() {
        // Not zero chunks — `read_chunked_field` needs a `:count` of at
        // least 1 to distinguish "field written as empty" from "field never
        // written at all" (`Ok(None)`).
        assert_eq!(chunk_value(""), vec![""]);
    }
}
