# MuxBus cloud sign-in: scope the keychain tokens to the channel

**Status:** implemented in the PR that adds this spec.
**Date:** 2026-10-02
**Owner:** Manoz
**Related:** `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (isolated auth is the default for every non-`stable` channel), `PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03.md` (the chunked Windows layout).

## 1. Problem

An AgentMux cloud (MuxBus) sign-in is meant to belong to one channel. Each local build, dev branch and portable runs its own channel with isolated auth. Only half of the sign-in actually did:

| Part of the sign-in | Where it lived | Scope |
|---|---|---|
| The `db_muxbus_credentials` row: whether the channel is signed in, plus cloud domain, client id, expiry, email and sub | the channel's own `identity-store.db` (`registry::paths::resolve_shared_store_path`) | per channel |
| The tokens (access, refresh, id) | OS keychain under the fixed names `acct:muxbus:global:*` (`storage/muxbus.rs`, `MUXBUS_KEYCHAIN_ID = CREDENTIAL_ID`) | **per host** |

Effects, all observed or reproduced on Area54 (2026-09-29 and 2026-10-02):

- **Sign-in in one channel replaces every other channel's tokens.** With the same account and the same cloud this happens to keep working, because the other channels just use the newer tokens. With a different account or cloud (for example a test instance on a new Cognito pool), the other channels silently run on that account's tokens while their UI still shows the old email.
- **Sign-out in one channel signs out all of them.** `muxbus_clear` deletes the shared entries. The other channels still have their SQL row, so they believe they're signed in, but their tokens are gone and their cloud connection fails until they sign in again.
- **A test instance can't safely use the cloud at all** on a machine whose main instance matters. That's why the M2.7 migration test D was cancelled.

Only `--headless` avoided this, by switching the whole secret store to files (`headless.rs`).

## 2. Fix

Put the tokens in the same scope as the row.

- `keychain_namespace()` returns `muxbus:channel:<AGENTMUX_CHANNEL>` exactly when the row is per channel: isolated auth is on **and** `AGENTMUX_INSTANCE_DIR` is set. Otherwise it returns `muxbus:global`. That's the same condition `resolve_shared_store_path` uses, so the row and the tokens can never land in different scopes. Channel names are unique per instance dir: `channels/<channel>/`, and `dev-<branch>[-<clone>]` for dev.
- Every key derives from the namespace: the blob (`<ns>`), the split fields (`<ns>:access|refresh|id`), and their `:N` / `:count` / `:gen` entries. Load, save and clear all compute it once per call.
- `stable`, and every non-isolated channel, keeps exactly the old names (`muxbus:global…`). Installed users see no change and need no migration. A unit test pins those names.

### 2.1 Channels that were already signed in

A channel that signed in before this fix has its row but nothing under its own namespace. On load, once every source under its own namespace (both layouts and the legacy plaintext columns) has come up empty, it **adopts the host-wide set once**:
- It reads `muxbus:global` in either layout, read-only.
- It copies the tokens into its own namespace (when the call allows migration writes).
- It returns them.

The global entries are never written or deleted from a channel namespace. So the `stable` channel, and any other channel still waiting to adopt, keep theirs. From that channel's next save on, its tokens are its own. A channel without a row never reaches this path, so a fresh channel still starts signed out.

## 3. Non-goals

- The broker's in-process credential id (`crate::muxbus::CREDENTIAL_ID`, used by the refresh scheduler and in logs) stays `muxbus:global`. It's a process-local name, not a keychain entry.
- Cleaning up a host-wide set left behind once no channel uses it. It's harmless, `stable` may still be using it, and a later `stable` sign-out deletes it as before.
- Other `secret_store` users, such as identity-account secrets. They're already keyed by account id, and per-channel isolation of those is `identities_dir`'s concern.

## 4. Testing

- **Unit tests (CI):**
  - The global names are unchanged.
  - The namespace rule matches the row rule: stable, isolated with an instance dir, isolated without one, and no channel.
  - Two channels, and a channel and the global set, never share any keychain entry: blob, field, chunk, count or generation.
- **Live test against the real OS keychain:** `per_channel_keychain_live`, `#[ignore]` because CI has no keychain. It writes chunked sessions (over 1000 characters, so multiple chunks) under two throwaway channel namespaces, checks that each reads back its own, clears one, and checks the other is still intact. It never touches `muxbus:global`. Run it on a dev machine with `--ignored`.
- **Manual check on Area54:** after this ships, two portables on different channels can sign in and out independently. Credential Manager then shows `acct:muxbus:channel:<channel>:…` entries next to the existing `acct:muxbus:global:…` ones.
