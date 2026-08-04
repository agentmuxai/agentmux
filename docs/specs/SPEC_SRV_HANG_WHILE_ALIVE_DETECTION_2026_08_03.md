# SPEC: srv hang-while-alive detection (#942 family)

**Date:** 2026-08-03
**Status:** Implemented
**Tracking:** #942 ("Service Supervision & Recovery"); closes the named
non-goal in `SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md`: *"srv
hang-while-alive detection (this covers exits; liveness probing for srv is
a separate follow-up in the #942 family)."*
**Scope:** `agentmux-launcher/src/supervisor/windows.rs` (Windows only —
see Non-goals)

## Problem

srv exit-triggered recycle (#2107) recovers cleanly from a crashed srv, but
nothing detects srv staying **alive but wedged** — a deadlock, an exhausted
blocking-thread pool, or a stuck `await` inside srv's own async runtime.
`srv_child.wait()` never resolves in that case, so the existing
crash-recycle machinery never fires, and the user is left with a session
that silently stopped responding for no visible reason.

## Design — probe, then reuse the existing recycle path

The host solves the analogous problem (a wedged CEF UI thread) with a pair
of modules: `ui_liveness.rs` (fire-and-forget pipe probe + nonce-matched
reply tracking) and `teardown_backstop.rs` (an armed state machine guarding
against legitimate zero-window states). Neither complication is needed for
srv:

- srv already exposes a **synchronous, unauthenticated HTTP health
  endpoint** (`GET /` → `health_handler`, `agentmux-srv/src/server/mod.rs`,
  mounted outside `auth_middleware`). A single bounded HTTP round-trip per
  tick gives a pass/fail answer within that same tick — no cross-tick nonce
  matching required.
- There is no "legitimate absence" state to guard against the way
  `teardown_backstop` must (host startup, crash-restart gap): srv is
  expected to answer whenever its process is running.

So this is one small module, `srv_liveness.rs`, with a single struct
(`SrvLiveness`: `consecutive_misses` + `last_alive`) and a process-global
singleton, following `ui_liveness.rs`'s exact conventions (same
`OnceLock<Mutex<>>` pattern, same "each test owns its own instance" reason
for avoiding cross-test interleaving).

**Recovery reuses #2107 unmodified.** On `SRV_HANG_REQUIRED_MISSES` (3)
consecutive missed probes, the new `select!` arm calls
`srv_child.start_kill()` and resets its own counters. The very next loop
iteration's existing `srv_status = srv_child.wait()` arm sees the exit and
runs the already-shipped respawn/rebind/host-recycle path exactly as it
would for a real crash. This module's only responsibility is deciding
"treat this as a crash" — it does not touch recovery logic, matching the
#942 program's prime directive that the supervisor stay "passive, bounded,
and simpler than everything it supervises."

No misclassification risk: `TerminateProcess` produces a generic non-zero
exit code, not the specific OOM exception code `classify_host_exit` checks
for, so a hang-triggered recycle naturally buckets as `Abnormal` and counts
against `SRV_RESTART_BUDGET` — exactly like a real crash. No
`srv_recycle_kill`-style flag is needed (that flag exists on the *host*
side, to stop a deliberate srv-triggered host recycle from being misread as
a GPU/OOM host fault — a different problem this doesn't touch).

### Why `tokio::net` instead of the `second_instance.rs` blocking-`std::net`
precedent

`second_instance::forward_open_new_window` is the existing hand-rolled
HTTP-over-loopback template (deliberately not `reqwest` — keeps the
launcher binary tiny for a fixed one-line request). `srv_liveness::probe`
follows the same request-construction style but uses `tokio::net::TcpStream`
+ `tokio::time::timeout` rather than blocking calls: `forward_open_new_window`
runs once, before the supervisor loop starts; `probe` runs every 10s
*inside* the same `select!` loop that also detects host/srv exit and the
teardown backstop, so a blocking call would stall all of those for up to
the probe timeout on every hiccup.

## Constants

- `SRV_PROBE_INTERVAL` = 10s (first tick delayed one interval, same
  convention as `ui_probe_interval`)
- `SRV_PROBE_TIMEOUT` = 3s
- `SRV_HANG_REQUIRED_MISSES` = 3 — bounds worst-case wedge→recycle latency
  to roughly 3 probe intervals plus timeouts (~30s), the same order of
  magnitude as the host teardown backstop's 30s grace.

`srv_liveness::reset()` is called at both `srv_spawner::spawn_srv` call
sites (cold boot and the exit-triggered recycle branch) so a freshly
spawned srv never inherits its predecessor's miss count.

## Non-goals

- **Unix.** `SRV_RESTART_BUDGET`/`SRV_RESTART_WINDOW` (the machinery this
  plugs into) are `#[cfg(target_os = "windows")]` today — srv
  crash-recycle itself isn't ported to Unix yet. Building hang detection on
  top of a platform where the underlying recycle path doesn't exist would
  be dead code; Unix parity is a follow-up once srv recycle lands there.
- A new srv-side debug endpoint to simulate a wedge for testing. A real
  process suspend (`pssuspend`, Sysinternals) is a more faithful test of
  "alive but not scheduled" than a busy-loop handler, and avoids adding
  permanent test-only surface to srv.

## Verification

- `cargo test -p agentmux-launcher` — `srv_liveness::tests` (pure
  miss-count/recycle-decision unit tests + three `probe()` integration
  tests against a local `tokio::net::TcpListener`: 200 response, stalled
  response past timeout, connection refused). Full launcher suite (213
  tests) green, no regressions.
- Live, on `task dev` (Windows):
  - Boot normally; confirm no `[srv-liveness]` miss logs during healthy
    operation.
  - `pssuspend <srv_pid>` to freeze srv without killing it.
  - Launcher log shows miss logs every ~10s, then after 3 (~30s) `srv
    wedged — forcing recycle`; srv is killed; the existing #2107
    respawn/rebind/host-recycle sequence fires (same "Restoring session…"
    splash as a real crash); windows reproject correctly; zero stray
    processes afterward.
  - Repeat to confirm `SRV_RESTART_BUDGET` (3/120s) still exhausts
    correctly — this change must not regress #2107's already-verified
    budget behavior.
  - Regression: a genuine srv crash (kill by PID, not suspend) still
    recycles exactly as before.
