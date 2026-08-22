# Retro: a stuck Keychain consent prompt could hang a caller indefinitely

**Date:** 2026-08-20
**Area:** `agentmux-srv/src/identity/secret_store.rs`
**Context:** follow-up to
[retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md](retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md)
§5, explicitly scoped out of that PR pending its own pass — this is that
pass.

---

## 1. Symptom (as reported / observed)

A signed, notarized DMG install triggered the MuxBus Keychain-prompt fix's
self-heal migration, which never completed. Live-reproduced during manual
verification: a diagnostic test reading the same real, pre-existing
chunked Keychain entries blocked for a very long time (its own progress
heartbeat fired past 60s; the reported total, ~22110s, is unreliable due to
this session's own tooling clock skew across a resume boundary, but "genuinely
blocked well past a minute" is solid) before eventually resolving as
`"User canceled the operation."`

## 2. Root cause

Every `identity::secret_store` operation (`get`/`get_optional`/`put`/
`delete`) is a synchronous call into the `keyring` crate, which on macOS
talks to Keychain Services / `SecurityAgent`. The first time a given
Keychain entry is accessed by a code signature the OS doesn't already
trust, this can require an interactive "App wants to access your
confidential information" consent dialog — and that call has **no
cancellation mechanism**. If the dialog isn't shown (headless/no attached
display session), isn't noticed, or is on a different Space, the
underlying platform call just blocks forever. Nothing in `secret_store`
bounded how long a caller waits for it.

This is worse than an annoyance: several call sites (most notably
`muxbus.rs`'s `muxbus_load_impl`/`muxbus_save`/`muxbus_clear`) hold a
mutex (`muxbus_save_lock`) for the entire keychain-read/write sequence — a
stuck read doesn't just block its own caller, it blocks every other
operation waiting on that same lock too.

## 3. Fix

Added a 15-second timeout, applied inside `secret_store`'s existing
`get`/`get_optional` functions themselves — no signature change, no caller
updates needed anywhere in the codebase. Each function now runs the real
platform call on a detached `std::thread`, and the caller waits on a
channel with `recv_timeout`. A timeout produces the same `Result<_, String>`
error shape these functions already returned for other keychain failures
(locked keychain, no Secret Service daemon, etc.), so every existing
caller's error handling — already written to tolerate "the keychain read
failed" — covers this for free.

This is a wait-bound, not a true cancellation: the detached thread doing
the actual blocked platform call keeps running until the OS resolves it
(answered or not), it's just no longer the caller's problem. A single
leaked thread per stuck prompt is a far better failure mode than an
indefinitely blocked, lock-holding caller.

Verified live against this exact machine's real stuck entries: the same
read that previously blocked for minutes-plus now returns after exactly
15.006s with a clear timeout error, unblocking the MuxBus self-heal
migration path to actually run on its next attempt.

**`put`/`delete` deliberately do NOT get this timeout** (reagent + Codex,
independently, in review round 2 of the PR that shipped this). A read is
safe to bound because it has no side effect — if the timeout fires, the
detached thread's eventual result is just discarded. A write is not: a
caller that treats a timed-out `put`/`delete` as failure and proceeds
(skipping a dependent DB write, or a rollback writing something else to
the same entry) can have the orphaned original write land afterward and
silently clobber whatever the caller did next, with no ordering guarantee
between them. There's no cancellation-equivalent compensation available —
the platform call truly cannot be cancelled — so until one exists, writes
keep the original unbounded (safe-if-slow) behavior. A stuck consent
prompt on a write still hangs its caller indefinitely; only reads are
protected by this fix.

## 4. Why 15 seconds

Long enough that a real user looking at their screen has a fair chance to
notice and answer a genuine first-time consent prompt; short enough that
an unanswered one doesn't leave a lock-holding caller wedged for an
unreasonable time. Not derived from a specific measurement — a round,
generous number chosen for this first pass. If it proves too short/long
in practice, it's a single constant (`secret_store::TIMEOUT`) to tune, not
a design to revisit.

## 5. What this does not fix

- **Writes (`put`/`delete`) are still unbounded** — see §3's last
  paragraph. A stuck consent prompt on a first-ever write (e.g. the very
  first `muxbus_save` after a login) can still hang its caller
  indefinitely. Only reads are protected today; making writes safe needs
  an actual cancellation or ordering-preserving compensation mechanism,
  not just a wait-bound, and wasn't designed in this pass.
- The underlying Keychain ACL weirdness/ACL-per-entry mechanics from
  [retro-macos-keychain-credential-isolation-gap-2026-08-17.md](retro-macos-keychain-credential-isolation-gap-2026-08-17.md)
  and the "Always Allow doesn't work" experience are unchanged — this
  makes an unanswered prompt fail fast, it doesn't make prompts stop
  happening or make consent decisions persist better.
- No way to actually cancel the underlying blocked platform call — see §3.
  Repeated stuck prompts (e.g. a sweep loop retrying the same doomed read
  every interval) will leak one thread per attempt. Not a concern at
  today's call volumes, but worth remembering if this pattern shows up in
  a tight retry loop later.

## 6. Follow-up

The §5 leaked-thread-per-retry risk materialized in practice two days
later — see
[retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md](retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md):
`cloud_subscriber.rs`'s independent retry/sweep timers each leaked their
own thread on every retry against the same unanswered prompt, accumulating
to 38 stuck threads on one real instance. That retro has the follow-up
recommendation (de-duplicate in-flight reads / cooldown after a timeout).

Otherwise: revisit the 15s constant if it proves wrong in practice (§4).
