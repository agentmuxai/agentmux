# SPEC: srv supervision via host recycle (#942 Phase 2)

**Date:** 2026-07-11
**Status:** Ready for implementation
**Tracking:** #942 Phase 2 ("srv supervision"); supersedes that spec's
`SrvManager` sketch for the srv case
**Scope:** `agentmux-launcher/src/supervisor/windows.rs` (+ unix mirror later)

## Problem

An srv exit still terminates the entire launcher (`supervisor/windows.rs`'s
srv-wait arm: log + `break 1`), taking the host — and the user's whole
session — down with it. This is the explicitly-documented Phase-2 gap
("srv is NOT yet supervised").

## Design — recycle, don't rewire

The original #942 sketch ("bespoke SrvManager; host tolerates srv
reconnect") predates crash-reproject. Teaching a live host to re-resolve
new srv endpoints means new IPC, new frontend reconnect states, and a
consistency story for every in-flight RPC — a large surface. But the host
is **disposable now** (Pillar 1 Step 4): killing and respawning it costs
seconds and self-restores the window set with the restoring-session
overlay. So srv supervision composes two baked mechanisms instead of
building a third:

1. **srv exits unexpectedly** (host still running) → check the srv restart
   budget (`3 per 120s`; exhaustion = fatal dialog + today's behavior).
2. **Respawn srv** via the same `srv_spawner::spawn_srv` call cold start
   uses — same pipe name (keyed on dir_hash, freed by the dead srv), fresh
   dynamic ports, durable SQLite state intact (WAL handles the crash).
3. **Rebind `srv_result`** to the new endpoints. This is the entire
   "rewire": host endpoints flow exclusively through
   `spawn_host_supervised(..., &srv_result, ...)` env at spawn time.
4. **Deliberately kill the host** (`start_kill`). It is unusable anyway —
   every backend connection it holds points at a dead process. The next
   select iteration's host-wait arm sees an abnormal exit and takes the
   EXISTING supervised-restart path — splash with "Restoring session…",
   new host spawned against the new endpoints, crash-reproject rebuilds
   the windows from srv's durable topology.

A launcher-inflicted recycle kill is flagged (`srv_recycle_kill`, consumed
once) so the host-restart arm skips the deterministic-crash classification
(`last_abnormal_code`/`host_degraded`): the host did not fault, and a
recycle must not step the retry ladder down to `--disable-gpu`. It still
COUNTS against the host restart budget — a crash-looping srv should
exhaust budgets and give up loudly rather than churn forever (defense in
depth on top of the srv budget).

## Non-goals

- Host tolerating srv reconnect in place (superseded — see above).
- macOS/Linux parity in this PR (unix.rs mirrors after Windows bake, the
  program's standard sequence).
- srv hang-while-alive detection (this covers exits; liveness probing for
  srv is a separate follow-up in the #942 family).

## Verification

- Budget unit-tested via the same helper the host budget uses.
- Live: kill srv by PID mid-session with windows open → launcher log shows
  srv respawn + deliberate host recycle; restoring-session splash; windows
  reproject; frontend fully functional against the NEW srv (open/close a
  window to prove RPC round-trips); `Client.windowids` intact. Second srv
  kill within the window exercises the budget arithmetic; a third exhausts
  it → fatal dialog, clean teardown, zero stray processes.
