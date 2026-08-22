# Retro: an unanswered Keychain prompt leaks one stuck thread per retry, not just one

**Date:** 2026-08-22
**Area:** `agentmux-srv/src/muxbus/cloud_subscriber.rs`, `agentmux-srv/src/identity/secret_store.rs`
**Context:** the predicted-but-untriggered risk flagged in
[retro-secret-store-keychain-read-timeout-2026-08-20.md](retro-secret-store-keychain-read-timeout-2026-08-20.md)
§5's third bullet ("a sweep loop retrying the same doomed read every
interval will leak one thread per attempt... worth remembering if this
pattern shows up in a tight retry loop later") — it showed up.

---

## 1. Symptom (as observed live)

A running `0.55.16` instance (already-trusted, per
[retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md](retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md),
so this is not that issue recurring) became unresponsive while its user was
exercising an authentication flow. A macOS "`security` wants to use your
`login` keychain" consent dialog was on screen and not going away.

`sample`d the instance's `agentmux-srv` process (PID 76270) for 2s:
**38 of its 59 threads** were parked inside
`SecKeychainFindGenericPassword` → `Security::KeychainCore::ItemImpl::...`
— all blocked on the same class of call, none of them the original request
tied to the visible dialog.

## 2. Root cause

`secret_store::get`/`get_optional` (the 2026-08-20 timeout fix) bounds how
long a *caller* waits — 15s — but cannot cancel the underlying blocked
platform call, by design (§3 of that retro): the detached `std::thread`
keeps running until the OS resolves it, leaking exactly one thread per
attempt. That retro's own §5 called this an acceptable one-off cost,
explicitly flagging it "not a concern at today's call volumes... worth
remembering if this pattern shows up in a tight retry loop."

`cloud_subscriber.rs` has more than one independent periodic caller into
this same read path:

- the connection loop's own `muxbus_load` call, retried on a backoff timer
  after any failure (including a timeout);
- the broker scheduler's background freshness sweep, which calls
  `muxbus_is_fresh` on its own independent interval, also reading through
  `secret_store`.

Each is written correctly in isolation — `spawn_blocking` so a stuck native
call doesn't stall a tokio worker (the fix for reagent's P1 on #2260), and
a timeout so a stuck native call doesn't stall the *caller* indefinitely
(the 2026-08-20 fix). Neither one, individually or combined, checks whether
the *previous* attempt at the same read is still in flight before starting
a new one. A timeout is reported back as an ordinary failure, the backoff
loop logs "retrying with backoff" and tries again next interval — spawning
a brand-new detached thread while the old one is still parked, unresolved,
in the exact same blocking call. Over the hours this instance had been
running with the dialog unanswered, these two independent timers
compounded into 38 concurrently leaked threads, enough native OS threads
blocked in Security framework calls to degrade the whole process to
"unresponsive" from the user's side — a far worse failure mode than the
single stuck caller the 2026-08-20 fix was written to prevent.

## 3. What this is NOT

- Not the 12-entry storm (#2665) — this instance's MuxBus data is already a
  single blob.
- Not the per-build-identity issue (2026-08-21 retro) — this is the same,
  already-trusted `0.55.16` build that had been running all day; the
  dialog here is plausibly a legitimate one-time consent request for a
  *different* Keychain item (a new/different account under test during the
  "authentication" work in progress), not a repeat prompt for
  `muxbus:global`. That part of the flow is not itself a bug.
- Not a data-corruption risk like the `put`/`delete` unbounded-write
  concern from the same 2026-08-20 retro §3 — reads are safe to retry
  indefinitely from a correctness standpoint. This is a **resource
  exhaustion** bug, not a correctness one.

## 4. Fix applied (live remediation only, not a code fix)

Force-killed the four processes backing that instance (`agentmux-srv`,
`agentmux-cef`, `agentmux-launcher`, and the `claude` CLI child) by PID and
relaunched the app bundle fresh. That drops every leaked thread at once.
The next read triggers exactly one fresh consent dialog; answering it
(and the earlier fixes already landed) means it shouldn't repeat for that
account/build going forward.

No code change shipped in this pass — this is a write-up of a confirmed,
live-diagnosed gap, not yet a fix.

## 5. Follow-up

- **Recommended, not yet built**: de-duplicate concurrent/rapid-retry
  attempts at the same underlying read instead of letting each timeout
  spawn an independent detached thread. Two shapes worth considering:
  - an in-process "this account_id's read is already in flight" guard —
    if a prior attempt hasn't resolved yet, later callers await/reuse that
    same pending result instead of starting a new OS call; or
  - a short cooldown after a timeout — skip the read entirely for some
    window after a timed-out attempt, rather than retrying on the very
    next backoff/sweep tick, so an unanswered prompt degrades to "one
    stuck thread, checked on rarely" instead of "one new stuck thread
    every retry interval, forever."
- Either fix should be designed once, in `secret_store` itself if
  possible, rather than per-call-site in `cloud_subscriber.rs` — any other
  periodic caller into `get`/`get_optional` (the broker sweep for other
  credentials, anything added later) has the identical exposure.
- Same open items carried forward from the 2026-08-20 retros: writes are
  still unbounded, and the broader `secret_store` call-site audit still
  hasn't happened. This finding is another data point for why that audit
  is worth doing rather than continuing to find these one at a time, live,
  on a real machine.
