# Retro: "Always Allow" didn't stop the Keychain prompt on app launch

**Date:** 2026-08-19
**Area:** `agentmux-srv/src/backend/storage/muxbus.rs`, `agentmux-srv/src/muxbus/cloud_subscriber.rs`

---

## 1. Symptom (as reported)

A freshly built, signed and notarized v0.55.15 `.dmg`, opened normally:
macOS prompted for the Keychain password on launch — the exact prompt
fixed once already (see
[retro-startup-keychain-prompt-model-catalog-2026-08-18.md](retro-startup-keychain-prompt-model-catalog-2026-08-18.md)),
now recurring. Clicking "Always Allow" did not make it stop — it kept
reappearing.

## 2. Investigation

- Confirmed the earlier fix (PR #2654, `allow_keychain_fallback` gating the
  model-catalog RPC's keychain fallback) was genuinely present in the built
  binary. It was — but it only ever covered one specific automatic keychain
  touch. It was never the only one.
- Traced every other automatic (non-user-triggered) startup path that
  touches AgentMux's own keychain service (`identity/secret_store.rs`,
  service `"agentmux"`). Found `CloudSubscriber::init_global`
  (`agentmux-srv/src/bootstrap.rs:997`), called unconditionally on every app
  launch, which spawns a background loop that calls `Store::muxbus_load()`
  immediately to check for a storable MuxBus cloud session to reconnect —
  a completely separate, legitimate (not cosmetic) automatic keychain read.
- Dumped the real macOS Keychain (`security dump-keychain`) and found live
  MuxBus token data on this machine, split across **12 separate keychain
  entries**: three token fields (`access`/`refresh`/`id`) × four sub-entries
  each (`chunk 0`, `chunk 1`, `count`, `gen`) — the chunked-per-field layout
  `muxbus.rs` uses.
- Read the chunking design's own history in the file's doc comments: it
  exists because Windows Credential Manager caps a single entry's blob at
  1280 real characters (`CRED_MAX_CREDENTIAL_BLOB_SIZE / 2`, confirmed via
  two rounds of live Cognito-token testing,
  `docs/specs/PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03.md`) — but
  the same comment says macOS/Linux have **no comparable cap**, and the
  chunked layout was applied to every platform uniformly anyway, purely for
  code-path consistency.
- This explains "Always Allow doesn't work": it isn't broken — it's
  literally answering one of up to 12 distinct OS-level consent decisions.
  Granting trust for entry #1 just surfaced entry #2's own prompt next,
  which looks indistinguishable from the click not registering.
- Verified live on `task dev` (a fresh build with the fix applied, run from
  an isolated worktree/branch so it didn't disturb the main dev session):
  confirmed the fixed binary compiles and runs, and separately confirmed via
  a targeted `cargo test` against the *real* keychain that a read requiring
  interactive consent, issued from this environment, blocks for a long time
  (the test's own progress heartbeat fired past 60s) before eventually
  failing with `"User canceled the operation."` — this execution context
  apparently cannot reliably surface/answer that dialog itself, but a real
  GUI launch does show it to the user (confirmed live: closing the
  worktree's dev-mode GUI instance also triggered a visible prompt).

## 3. Root cause

Two compounding issues:

1. **12 separate keychain entries for one logical credential**, all written
   uniformly across platforms for a size constraint ([1280-char Windows
   Credential Manager cap]) that only applies to Windows. On macOS, each
   entry is its own independent OS access-consent decision — the more
   entries, the more prompts, and per-entry trust doesn't "add up" to
   whole-credential trust the way a user would expect from "Always Allow."
2. **A keychain read requiring interactive consent can block for a long
   time rather than failing fast** — nothing in `secret_store`/`muxbus.rs`
   wraps these reads in a timeout. They already run on `spawn_blocking`
   threads (to avoid stalling the async runtime), a pattern several past
   reagent reviews on this exact file added specifically because these
   reads *can* hang (documented for Linux's Secret Service D-Bus daemon) —
   but nothing bounds how long that hang can last. `muxbus_load_impl` holds
   `muxbus_save_lock` for the entire read, so a stuck read can block other
   muxbus operations too.

## 4. Fix (this round)

Collapsed macOS/Linux MuxBus token storage from the chunked 12-entry layout
to a single combined JSON blob under one keychain entry — reusing the
already-existing "legacy blob" format and its migration-source machinery,
just pointed at a different direction than it was built for. Windows keeps
the untouched, still-necessary chunked layout. A pre-fix macOS/Linux install
(with old chunked entries already on disk, like this exact machine)
self-heals on the next successful read: reads the old entries once, writes
them as a single blob, deletes the twelve old ones.

Net effect: at most **one** Keychain consent decision instead of up to
twelve, and it actually persists across launches once granted.

## 5. What this fix does NOT address

Item 2 in §3 — the unbounded-block risk — is real and not fixed here.
Reducing to one entry means at most one hang instead of up to twelve, but a
single stuck read can still block indefinitely if its consent prompt is
never answered (not shown, dismissed, or the process has no way to surface
one). Fixing that properly means wrapping keychain reads in a bounded
timeout, which touches shared `identity/secret_store.rs` plumbing used well
beyond MuxBus (Armory accounts, the earlier model-catalog fix, etc.) —
scoped out of this change deliberately, per explicit direction, as a
separate follow-up.

## 6. Follow-up

- Add a bounded timeout around `secret_store` keychain reads (and
  `muxbus_save_lock`-guarded call sites specifically) so an unanswered
  interactive consent prompt degrades to a graceful "couldn't read"
  instead of hanging the calling path indefinitely. Not started — needs
  its own scoping pass given the shared-plumbing blast radius.
- The still-unexplained hash-suffixed `Claude Code-credentials-<hash>`
  Keychain entries from
  [retro-macos-keychain-credential-isolation-gap-2026-08-17.md](retro-macos-keychain-credential-isolation-gap-2026-08-17.md)
  §5 remain open — unrelated CLI-vendor Keychain usage, not AgentMux's own,
  but still unexplained.
