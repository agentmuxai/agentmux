# Plan — fix MuxBus token persistence on Windows (Credential Manager 2560-byte cap)

**Date:** 2026-08-03
**Status:** proposed → implementing in this same PR
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

### Known residual limit

If any *single* token itself exceeds ~2560 bytes on Windows, the write for
that one field will still fail — but the error will now name exactly which
field, rather than obscuring it inside a combined blob. Not fixing this
further here: real-world Cognito tokens don't approach that size
individually, only combined.

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
