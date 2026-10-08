# Retro: MuxBus kept signing itself out (2026-10-08)

**Status:** retro
**Trigger:** the operator: "figure out why muxbus keeps logging out automatically."
**Outcome:** root-caused from the installed instances' logs and the source. Fixed by
`SPEC_MUXBUS_SIGN_IN_PER_CHANNEL_2026_10_08.md`: a channel signs in on its own, and
sign-in reads and writes are one critical section across processes.
**Author:** Camper

## TL;DR

Several AgentMux processes on one machine shared a single MuxBus sign-in in the OS
keychain, and the only lock around saving it was inside each process. When two of them
refreshed the sign-in at the same moment, their writes interleaved and left the three
stored tokens from different saves. Every later read saw that, treated it as "no sign-in
stored", and the refresh code treated "no sign-in" as permanent. The result was a sign-out
a few hours after each sign-in, until the next one.

## What the operator saw

MuxBus signed out on its own, several times a day, with nothing in the UI to say why. Each
sign-in held for a few hours.

## Evidence

Source: the installed launcher log, which carries every srv process's stderr, from Oct 6 to
Oct 8. Times are UTC.

- The log holds about 4,500 lines of `muxbus: split keychain entries have mismatched
  generation stamps (a torn write from an interrupted save)`, from five srv processes of
  versions 0.59.11 to 0.59.15 running at once.
- Each sign-in is followed, hours later, by the same sequence: the mismatch warning, then
  `token refresh failed error=no refresh_token stored`, then `credential needs a fresh login,
  no longer auto-retrying`, repeating every minute from every process until the next
  sign-in. Sign-ins were at Oct 7 16:30 and 19:37 and Oct 8 02:40 and 06:59; the mismatch
  runs were from about Oct 7 22:00 to Oct 8 02:40, and from Oct 8 14:22.
- The first mismatch ever logged (Oct 6 20:27:08.308) comes from one process 54 ms before
  another process reports the sign-in fresh. On Oct 8 the run started at 14:22:16.305, 70
  ms after two processes (v0.59.15 and v0.59.13) both reported it fresh.

- Reproduced on purpose afterwards: four processes saving and reading one sign-in in the real
  Windows keychain tore 35 to 59 of 160 reads without a cross-process lock, and none with it. A
  save took up to about 0.7 s, so collisions between processes were easy to hit.

## Root cause

Three things had to be true, and all were:

1. **The sign-in was host-wide.** Since 2026-10-03 every channel shared one keychain
   namespace, `muxbus:global` (`SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`). Before
   that only the installed releases did; every other channel had its own.
2. **A save is not atomic, and its lock is per process.** On Windows the sign-in is three
   fields, each chunked, plus counts and a generation stamp: about a dozen separate
   Credential Manager writes. `muxbus_save_lock` is a `Mutex` inside one `Store`, so two
   processes saving at once interleave, and a read in a second process can land mid-save.
   The code already knew this about threads (the comments on `muxbus_load_impl` explain
   it) and fixed it for threads only.
3. **A torn read became a permanent sign-out.** Mismatched stamps made the load return
   "nothing stored". The refresh closure turns an empty refresh token into a permanent
   failure ("only re-login fixes it"), and the broker stops retrying a permanent failure.
   A condition that should have been transient (a read that raced a write) was
   indistinguishable from a real sign-out.

Every process refreshes on its own schedule, and the access token lasts about an hour, so
with five processes the collisions were routine. The sign-in survived until a collision
tore it, then stayed torn.

## Why it wasn't caught

- The shared-auth spec's hazard table covered SQLite (WAL, busy timeout), schema skew,
  account folders and token rotation. The keychain was in its table as a thing that moves
  per channel, not as a write path with its own concurrency. "Two servers on the shared
  stores" was judged safe because it is "what `stable` and two live versions already do".
  For SQLite that is true; for the keychain's multi-entry write it isn't.
- The per-channel keychain spec (`SPEC_MUXBUS_KEYCHAIN_PER_CHANNEL_2026_10_02.md`) had
  already existed for a day and was reversed by the shared-auth spec the next day, in
  the same change as every other piece of auth. MuxBus sign-in was treated as one more
  piece of "auth" to share.
- There was no test with two processes. The existing tests exercise the torn state by
  hand-building it, which proves the reader handles it, not that nothing creates it.
- The signal was in the logs the whole time, as a WARN every minute from every process;
  nothing surfaced it.

## What we changed

(Details in the spec.)

1. **A channel is the unit of sign-in.** Every channel except `stable` signs in on its
   own: its own keychain entries, its own saved-login row, its own cached per-agent cloud
   credentials. Two channels can be signed in at once, to the same account or different
   ones, and neither touches the other's. `stable` keeps the host-wide sign-in, so
   installed releases stay signed in.
2. **One critical section across processes.** An OS advisory lock beside the shared store
   covers every sign-in save, load and clear, so two processes in one channel (two live
   versions of `stable`, say) cannot interleave. The wait is bounded; a stuck holder cannot
   stop sign-in.
3. **A torn read is retried before it means "signed out".** An older build with no lock
   may be mid-save; the read is repeated a few times first.

## What we did not change

- A tear that persists still means "sign in again". Repairing it automatically is
  possible (the refresh token does not rotate, so a torn set could be refreshed back to
  whole), but a set torn between two different accounts would pair one account's token
  with another's; the repair would have to verify the account first.
- Channels other than `stable` now need their own sign-in on first run, once per channel.
  That is the cost the shared-auth spec removed, and the point of the change.
- Older builds, which have neither the lock nor per-channel sign-in, can still tear the
  host-wide set while they run beside a newer one.
- Provider logins (Claude, Codex and the rest), accounts, Global Memory and bundles stay
  shared across channels; only MuxBus sign-in moves.

## Lessons

1. **A lock that only covers threads is not a lock on host-wide state.** Anything in the
   OS keychain, a shared file or a shared database is shared with every process that runs
   as this user. The check for "is it shared?" belongs in the design of the lock, not
   its comment.
2. **"Not found" and "found something broken" are different answers.** A torn read
   returned the first, so the caller made a permanent decision from a transient state.
3. **A decision that moves many things at once needs each piece checked against the
   reason it was separate.** The 2026-10-02 spec existed because a channel's sign-in and
   sign-out had been hitting every other channel's.
4. **A repeated WARN with a clear cause is a bug report.** A one-minute-cadence
   warning from five processes for hours should have been something an operator could
   see.
