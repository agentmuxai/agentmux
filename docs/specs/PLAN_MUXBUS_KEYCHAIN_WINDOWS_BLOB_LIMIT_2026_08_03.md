# Plan — fix MuxBus token persistence on Windows (Credential Manager 2560-byte cap)

**Date:** 2026-08-03
**Status:** implemented, live-verified working (§7), and a P1 review finding
on the chunking design fixed (§8).
**Context:** live-debugged on channel `local-main-b28b7a-9172ff88` (this
machine, Windows) while checking whether GitHub PR-review jekt notifications
were reaching that instance via the muxbus GitHub consumer
(`agentmux-cloud/muxbus/consumers/github/handler.ts`).

## 1. Symptom

`agentmux-srv` on this Windows instance logs, continuously, once a minute:

```
WARN srv:muxbus_handlers  muxbus: token expired, skipping injection — user should reconnect via muxbus.login
```

A fresh `muxbus.login` (PKCE flow) was performed live during this
investigation. The browser round-trip succeeded (`PKCE login succeeded`), but
the credential save that follows it failed, and the "token expired" warning
resumed within ~90 seconds:

```json
{"timestamp":"2026-08-03T15:23:54.426194Z","level":"WARN",
 "fields":{"message":"muxbus.login: failed to save credentials",
 "error":"muxbus: keychain write failed: keychain write failed: Attribute 'password' is longer than platform limit of 2560 chars"},
 "target":"agentmux_srv::server::muxbus_handlers"}
```

Net effect: `cloud_subscriber`'s poll loop (`GET /reactive/pending/:agent_id`)
never has a valid token to poll with, so **no muxbus-injected notification —
including GitHub PR-review jekts — can reach this instance**, no matter how
many times the user re-logs-in through the UI.

## 2. Root cause

`agentmux-srv/src/backend/storage/muxbus.rs` bundles all three Cognito tokens
(`access_token` + `refresh_token` + `id_token`) into one JSON blob
(`MuxBusTokens`) and writes it to a *single* OS-keychain entry keyed by
`crate::muxbus::CREDENTIAL_ID` (`"muxbus:global"`), via
`agentmux-srv/src/identity/secret_store.rs::put` (the `keyring` crate).

Windows Credential Manager's generic-credential blob (`CredWrite`'s
`CredentialBlob`) is capped at `CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560 bytes.
A Cognito `id_token` alone is a full JWT (often 800–1500+ bytes depending on
claims); combined with `access_token` and `refresh_token` plus JSON
punctuation/keys, the bundled blob reliably exceeds 2560 bytes on this
account. macOS Keychain and Linux Secret Service have no comparable limit,
so this is Windows-specific and was not caught when MuxBus tokens were moved
from plaintext SQLite columns to keychain-backed storage (see
`docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md:27-28`,
written just before that migration landed and still describing the old
plaintext-only design).

Compounding it: `muxbus_save` (`muxbus.rs:291-292`) `?`-propagates on a
keychain-write failure with no fallback, so **a fresh login on an affected
Windows machine can never persist a working token at all** — not even into
the legacy plaintext SQL columns, which are only ever populated by rows that
predate this keychain migration.

## 3. Fix

Split the single combined-blob keychain entry into three per-field entries,
each holding one raw token string — individually far under any platform's
size cap:

- `muxbus:global:access`
- `muxbus:global:refresh`
- `muxbus:global:id`

(`crate::muxbus::CREDENTIAL_ID` stays `"muxbus:global"` unchanged — it's also
the broker-scheduler registration key in `cloud_subscriber.rs`, out of scope
here. Only `backend/storage/muxbus.rs`'s internal keychain layout changes.)

### Backward compatibility

Existing macOS/Linux users already have a valid combined blob under the old
single key (`muxbus:global`) — this fix must not force them to re-login.
`muxbus_load_impl` gains a migration branch, same shape as the existing
legacy-plaintext-to-keychain migration already in this file: if the three
new split entries are absent but the old combined-blob entry is present,
read it, write its three fields into the new split entries, then delete the
old entry. Self-heals on first load after upgrade, no user-visible effect.

The existing legacy-plaintext (pre-keychain, `db_muxbus_credentials` SQL
columns) migration path is unchanged in shape — it now writes directly to
the three split entries instead of one combined blob.

### Save-path rollback

`muxbus_save`'s existing SQL-failure rollback (restore/delete the keychain
entry to match pre-call state) is per-entry today; with three entries it
becomes three independent restore/delete attempts, each already covered by
the existing `PriorKeychainState::{Existed,Absent,Unknown}` logic — apply
that logic three times (once per field) rather than inventing new rollback
semantics.

### Known residual limit (superseded — see §6)

If any *single* token itself exceeds ~2560 bytes on Windows, the write for
that one field will still fail — but the error will now name exactly which
field, rather than obscuring it inside a combined blob. Not fixing this
further here: real-world Cognito tokens don't approach that size
individually, only combined.

**This assumption was wrong** — see §6. The residual limit isn't just a
named-but-unfixed edge case; it reproduced immediately on the very first
live test.

## 4. Testing

- Existing `tokens_json_round_trips` unit test in `muxbus.rs` covers the
  legacy combined-blob JSON shape (kept, since old-blob migration still
  needs to deserialize it).
- Add a unit test asserting each split-entry write call receives a value
  under a conservative size budget for representative token lengths (can't
  exercise the real OS keychain in CI — this is a regression guard on the
  splitting logic itself, not an integration test against Credential
  Manager).
- Manual verification on this machine: `muxbus.login` on the fixed build,
  confirm `muxbus.login: failed to save credentials` no longer appears and
  `token expired, skipping injection` stops recurring after a successful
  login.

## 5. Out of scope

- The GitHub-consumer Lambda side (`agentmux-cloud/muxbus/consumers/github`)
  — already correct; this was purely a local token-persistence bug.
- Any change to `CREDENTIAL_ID` itself or the broker scheduler.
- General "keychain write failure has no fallback" hardening beyond this one
  call site — flagged here for awareness, not addressed.

## 6. Live test found the §3 design insufficient — chunking instead

Verified in an isolated `task dev` instance (`agent2-fix-muxbus-keychain-windows-blob-limit`
channel — separate data dir from the live channel this bug was found on, so
testing it couldn't disrupt the running session). A real `muxbus.login`
against this account produced the **exact same error** as before the
per-field split:

```
muxbus.login: failed to save credentials
error: muxbus: keychain write failed: keychain write failed:
       Attribute 'password' is longer than platform limit of 2560 chars
```

This wasn't the multi-channel/multi-process angle it first looked like (the
keychain keys were already global across every channel *before* this fix
too — that's unchanged and intentional, MuxBus is one human login shared by
every local instance). It's that §3's assumption — "real-world Cognito
tokens don't approach 2560 bytes individually" — was simply wrong for this
account: at least one of the three fields (most likely `id_token`, which
carries this account's custom claims) exceeds 2560 bytes **on its own**.

**Revised fix:** each of the three fields (`muxbus:global:access` /
`:refresh` / `:id`) is now further chunked into as many `<field>:0`,
`<field>:1`, … entries as needed (each safely under the cap, 1800-byte
budget vs. the 2560-byte limit), tracked by a `<field>:count` entry. This
removes the size assumption entirely rather than moving it — holds for any
token size, not just "sizes we've observed so far." `write_chunked_field`
also clears trailing chunks left over from a previous, longer value at the
same field, and the existing rollback-on-later-failure logic
(`PriorKeychainState`) now composes across every chunk + count entry touched
in a call, not just one entry per field.

Re-verification of this revision (same dev instance, fresh `muxbus.login`)
hit the **exact same error a third time** — see §7. Chunking itself wasn't
the missing piece; the chunk-size budget was.

## 7. Third round: the char/byte mixup in `keyring`'s own error message

With `MAX_CHUNK_LEN = 1800`, the same live dev instance produced the same
"longer than platform limit of 2560 chars" error again. Traced into the
vendored `keyring` crate source (`keyring-2.3.3/src/windows.rs:182`):

```rust
if password.encode_utf16().count() * 2 > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
```

`CRED_MAX_CREDENTIAL_BLOB_SIZE` is 2560 **bytes** — the real Win32
`CredWrite` limit. Windows credential blobs are UTF-16, 2 bytes/unit, so the
check is really `chars * 2 > 2560`, i.e. `chars > 1280`. But the error text
built in `error.rs:72` prints that same 2560 constant as if it were the char
limit — `"Attribute '{name}' is longer than platform limit of {len} chars"`
with `len = CRED_MAX_CREDENTIAL_BLOB_SIZE`, not `/ 2`. The crate's own error
message is off by 2x for this platform. `MAX_CHUNK_LEN = 1800` chars was
therefore ~40% over the *real* limit (1280) while looking like it had ~30%
headroom under the limit as reported.

**Fix:** dropped `MAX_CHUNK_LEN` to 1000 (well under 1280, real margin this
time). Re-verified on the same live dev instance: `muxbus.login` succeeded
and stayed connected — log shows `auth.broker.fresh: credential is fresh`
and `cloud_subscriber: WebSocket connected`, no `failed to save
credentials`, no further `token expired, skipping injection`. Confirmed
fixed.

## 8. Review finding: `read_chunk_count` swallowed read errors (P1, fixed)

reagent flagged (twice — unaddressed after the first flag on the prior
commit) that `read_chunk_count` collapsed a genuine keychain read *error*
into the same `0` result as "no `:count` entry exists yet." Two real
consequences:

- **`muxbus_clear` (logout):** used the count to bound `for i in 0..count {
  delete(chunk_key) }`. A transient count-read failure → `count = 0` → the
  loop deletes nothing — but the `:count` key itself is still deleted
  unconditionally right after. Logout appears to succeed while the real
  access/refresh/id token chunks are silently orphaned in the OS keychain —
  a genuine secret-hygiene bug, not just a logic nit.
- **`write_chunked_field`:** the same swallowed error meant a transient
  failure reading the *old* count skipped clearing stale trailing chunks
  left over from a previous, longer value.

**Fix:** `read_chunk_count` now returns `Result<usize, StoreError>`,
distinguishing "no entry" (`Ok(0)`) from "read failed" (`Err`).
`write_chunked_field` propagates the error via `?` rather than assuming 0 —
consistent with this file's established rule (see `PriorKeychainState`'s own
doc comment) that a real read failure must never be silently treated as
"nothing was there." `muxbus_clear` can't simply propagate (logout is
documented best-effort — a keychain problem must not block clearing the SQL
row), so instead it falls back to scanning a generous bound
(`MAX_PLAUSIBLE_CHUNKS = 32`, i.e. 32,000 chars — no real Cognito token
comes remotely close) when the real count can't be read.
`secret_store::delete` on a non-existent entry is already a no-op success,
so over-scanning past the real count is harmless; it guarantees actual
cleanup instead of silently skipping it.

## 9. Review finding: chunked writes broke the lock-free read path (P1, fixed)

reagent's next review pass on §8's fix caught a second, independent bug
introduced by chunking itself (not by anything in §8): `muxbus_is_fresh`
(called via `muxbus_load_impl(false)`, deliberately lock-free per the
existing `RefreshScheduler::register` contract) can run concurrently with a
`muxbus_save`/`muxbus_clear` write. With the OLD single combined-blob
layout that was safe — the OS keychain writes one entry atomically, so an
unlocked concurrent read could only ever see fully-old or fully-new data.
The chunked layout replaced that one atomic write with several sequential,
non-atomic ones (`write_chunked_field` puts chunk 0, chunk 1, …, clears
stale trailing chunks, then updates `:count`, no lock held from the
reader's side). An unlocked read landing mid-write can now see a torn mix
of old and new chunks for a multi-chunk token — every individual
`get_optional` still succeeds, so nothing errors, it just silently
reconstructs a corrupted string.

**Fix:** `muxbus_load_impl` now takes `muxbus_save_lock` unconditionally
(previously only when `allow_migration`), serializing every read against
`muxbus_save`/`muxbus_clear`'s writes. This does NOT reintroduce the
original concern that made `muxbus_is_fresh` lock-free in the first place
(reagent P2 on #2260) — that was about an unexpected *write* (a lazy-
migration self-heal) happening inside a nominally read-only freshness
check, not about lock acquisition; `allow_migration` still gates every
actual write, `muxbus_is_fresh` still performs none. Verified no deadlock:
`cargo test -p agentmux-srv --bin agentmux-srv muxbus` (15 tests, including
`cloud_subscriber`'s own suite) passes.

## 10. Review finding: cross-field write not atomic across a process crash (P2, fixed)

reagent's third pass, after §9's lock fixed the *same-process concurrent
read* case, flagged the remaining *cross-process-crash* case: each field's
own chunks+count are individually self-consistent (`write_chunked_field`'s
internal chunk+rollback atomicity), but `write_split_tokens` writes the
three fields sequentially with no lock surviving a process kill. A crash
between two fields' writes leaves one field holding its OLD value next to
the other two fields' fresh values — each field individually well-formed,
so `read_split_tokens`'s existing "all three present vs. some missing"
check doesn't catch it, and the result silently pairs tokens from two
different login sessions/accounts.

**Fix:** `write_split_tokens` now generates one `new_generation()` stamp
(nanosecond wall-clock, unique enough in practice) per call and passes it
to all three `write_chunked_field` calls, which each write it to their own
`<field>:gen` entry alongside their chunks/count. `read_split_tokens`
requires all three fields' generation stamps to match before accepting the
set as valid; a mismatch (or a missing `:gen` on an otherwise-complete
field) is treated the same as "not yet migrated," falling through to the
legacy sources or ultimately requiring a fresh login — the safe failure
mode, instead of silently mixing sessions. `muxbus_clear` deletes the
`:gen` entry alongside each field's chunks/count on logout.

## 11. Review finding: within-field write still not crash-safe (P1, fixed)

reagent's fourth pass caught that §10's fix didn't go far enough:
`write_chunked_field`'s own internal write order was still chunks → clear
stale trailing chunks → `:count` → `:gen`, LAST. A crash specifically
between the new-chunk-write loop finishing and the trailing-chunk-delete
loop finishing leaves `:count` still pointing at the OLD (larger) count,
while the leading chunk indices already hold NEW content and the trailing
ones still hold OLD content. On restart, `read_chunked_field` reads exactly
`old_count` chunks — none technically missing, so no error — silently
splicing a new-prefix/old-suffix value that is neither the previous nor
the new token. Because the crash lands before `:gen` is reached, this
field's generation stamp is untouched, so §10's cross-field equality check
never even sees a mismatch.

**Fix:** reordered `write_chunked_field` to delete `:gen` FIRST — before
touching any chunk content — and write the fresh `generation` value LAST,
only once the chunks and `:count` are fully consistent. `read_chunked_field`
already treats a missing `:gen` on an otherwise-populated field as an error
("keychain state is inconsistent"); with this reorder, a crash ANYWHERE
between the delete and the final write leaves the field in exactly that
detectable state, instead of a silently-reconstructible splice. The single
`PriorKeychainState` captured for `:gen` before the initial delete is
reused for the final write's own rollback path too (not re-captured, which
would incorrectly record "Absent" — the state WE just deleted it to — as
what a later failure should roll back to).

Live-verified once more end to end on the same dev instance after this
change: a fresh `muxbus.login` correctly rejected the OLD pre-generation-
stamp session (`"missing generation stamp... keychain state is
inconsistent"` — the intended, safe detection, not a bug) and required
re-login; the fresh login then succeeded (`PKCE login succeeded` →
`auth.broker.fresh: credential is fresh` → `cloud_subscriber: WebSocket
connected`).
