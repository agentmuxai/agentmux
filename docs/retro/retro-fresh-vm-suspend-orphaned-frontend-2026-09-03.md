# RETRO — VM suspend leaves a "running but frontend-less" AgentMux instance; double-click on the exe recovers nothing visible

**Date:** 2026-09-03
**Author:** Camper
**Severity:** Medium — no data lost, the instance's backend stayed healthy throughout, but the user's only recovery action (double-click the exe) produced no working window. On a real user's machine (not an agent-operated VM), this reads as "AgentMux is broken, I have to kill it in Task Manager."
**Area:** `agentmux-launcher` single-instance forwarding (`other_instances.rs`), the `pool_respawn_on_promote` saga (`saga/pool_respawn.rs`), `agentmux-cef`'s `promote_pool_window` (`commands/window_pool.rs`), Windows/VM power management.

---

## Summary

While testing the fresh-PC onboarding fix (#2942/#2943/#2947) on a real Windows 11 VM, the VM was left running unattended and went to sleep for ~4h15m (`Sleep Reason: System Idle`, confirmed via the Windows System event log). The running AgentMux instance's live WebSocket connection died silently at the moment sleep began — VM suspend doesn't deliver a graceful TCP close, so the backend (`agentmux-srv`) had no signal that the frontend was gone. When the user later double-clicked the exe expecting a fresh window, the launcher did exactly what it's designed to do — detected the still-running instance via the named pipe and forwarded an `open_new_window` request — but the window that request produced was **a pre-warmed "pool" window promoted from before the sleep**, whose own frontend connection was equally dead. The user saw nothing respond. Four overlapping top-level AgentMux windows now exist on screen at conflicting positions from repeated attempts, none of them confirmed working.

## Timeline (all times reconstructed from real logs — `agentmux-launcher.log` — and the Windows System event log, not inferred)

- **~12:53 local** — first clean launch of the app on this VM (separate, unrelated investigation of first-launch latency — see tracking issue #2940).
- **13:38:59** — `Microsoft-Windows-Kernel-Power` Event ID 42: *"The system is entering sleep. Sleep Reason: System Idle."*
- **20:38:57.020135Z** (`agentmux-launcher.log`) — `WebSocket client disconnected conn_id=b7f3c2a9-...`. This is ~1 second after the sleep event above (UTC/local offset accounts for the hour gap between the two timestamps) — the disconnect is the OS tearing down network state as the machine suspends, not a graceful app-level close.
- **20:38:59** (`[1788467940]`) — last launcher log line before a **4-hour-16-minute gap with zero log activity** — no `ui-liveness` probes, no `srv` stderr relayed, nothing. The launcher process itself was suspended along with the rest of the VM.
- **13:39:01 / 17:54:27 local** — `Microsoft-Windows-Kernel-Power` Event ID 107 (*"The system has resumed from sleep"*) and `Microsoft-Windows-Kernel-General` Event ID 1 (*system clock jumped from `20:39:01` to `00:54:27`*) — the VM woke back up.
- **`[1788483271]`** (log resumes) — `[ui-liveness] UI thread alive — probe nonce=39 rtt=4ms` — the launcher's liveness probe to the host succeeds again (host process survived the suspend, Win32 message loop responsive).
- **`[1788483273]` / `[1788483284]`** — `[srv-liveness] missed health probe (1 consecutive)` / `(2 consecutive)` — but the *backend* misses its health check right after resume. (It later recovers on its own — see Open Questions.)
- **`[1788483287]`** — a **new** `agentmux.exe` process starts (the user's double-click). `instance_claim` → `pipe bind failed (already_running=true): Access is denied.` → **`forwarded open_new_window to existing instance — exiting 0`**. This is the single-instance mechanism working exactly as designed (`agentmux-launcher/src/other_instances.rs`, invariant I4 in `CLAUDE.md`).
- Immediately after — the existing instance's saga machinery fires: `[saga] starting saga_id=1 name=pool_respawn_on_promote` → `IssueCmd::Host SpawnPoolWindow` → `Done — emitting SagaCompleted`.
- **`[1788483290]`** — a *new* WebSocket connects (`conn_id=75390d27-...`), `ws_setup` completes in 16ms — technically successful.
- **`[1788483331]`** (~40s later) — a burst of ~35 `ws egress lane full for conn 9ceeb40c-... — consumer stalled, dropping event` warnings, all for a *different* connection id than the one that just connected cleanly. `9ceeb40c` is not the fresh `75390d27` connection — it reads as a leftover, pre-sleep connection the server never learned had died, now finally being drained/discarded under backpressure.
- **Current state** (checked live via Win32 `EnumWindows` against the guest): **four** top-level windows titled `AgentMux` / `Window N - Tab 1 - AgentMux` exist, three of them at the identical rect `(0,0)-(1024,768)`, stacked on top of each other. Which one (if any) is actually rendering correctly has not been visually confirmed.

## Root cause

**Confirmed:** the VM's system sleep is what broke the running instance's live connections. This is not in dispute — the WebSocket disconnect timestamp and the `Sleep` event are ~1 second apart, and the entire 4h16m log-silence gap exactly matches the sleep→resume window.

**Strongly supported, not yet visually confirmed:** the promoted window the user's double-click produced is one of the pool's pre-warmed windows, created *before* the sleep, carrying a dead connection. Read directly in `agentmux-cef/src/commands/window_pool.rs`'s `promote_pool_window`:

```rust
// Validate browser is still alive. On non-Windows we don't cache a
// native window handle; CEF state presence is the liveness check.
if state.get_browser(&label).is_none() {
    ...
    cleanup_failed_promote_orphan_cross_platform(state, &label);
    return None;
}
```

The only "liveness" check before promoting a pool window is **"does this CEF browser handle still exist in the host's own state map"** — a process-object-existence check, not a functional one. A pool window's CEF browser object survives a VM suspend perfectly well (it's just memory); its WebSocket connection to the backend does not. Promotion doesn't re-verify the connection, doesn't force a reload, and doesn't wait for a fresh first-paint/connect signal before considering the `open_new_window` request satisfied — the saga (`pool_respawn_on_promote`) only tracks the pool *refill*, not whether the promoted window is actually usable.

`pool_respawn_on_promote`'s own doc comment (`saga/pool_respawn.rs:70-79`) already states the failure posture plainly: *"if refill genuinely fails, the next promote will start a fresh saga"* — the saga has no concept of "the promote itself produced a dead window," only "did the pool get refilled." That's a real, load-bearing gap for exactly this scenario.

## Why the user saw "nothing happens"

The launcher did not fail silently — it correctly forwarded the request and the host correctly ran its promote/refill machinery, logging success the whole way. The gap is entirely in what "success" means: the code confirms *a window object exists and the pool was refilled*, never *the window the user is looking at is actually connected and responsive*. From the user's side, double-clicking the exe either re-surfaced an already-dead window, or added a new dead one to the stack — visually indistinguishable from "nothing happened."

## Recommendation

Matches what was asked for directly: **at the very worst, double-clicking the exe should recognize this state and produce a working frontend, even if that means a second window.**

1. **Verify, don't just dispatch, on `open_new_window`.** After promoting a pool window (or spawning a fresh one) in response to a forward, wait for a bounded, real confirmation that it's alive — e.g. a fresh first-paint/WebSocket-connected event *after* the promote, not merely the pre-existing CEF-handle check. This is the same kind of signal `splash.rs`'s dismiss-on-`on_load_end` already depends on for the cold-start path; the promote path has no equivalent for the warm-restart path today.
2. **If that confirmation doesn't land within a short timeout, don't leave the user with the ambiguous result.** Fall back to spawning a genuinely independent, fresh top-level window (accepting a second/redundant window as the honest cost) rather than silently trusting a promote that might be handing back a corpse.
3. **Treat "system resumed from sleep" as a signal to invalidate the pool, not just as background noise.** The launcher already logs `srv-liveness` missed-probe events right after resume — that's a real, already-available signal. Tying pool invalidation (or at least a forced reload) to it would stop stale pre-sleep pool windows from ever being *offered* for promotion in the first place, closing the root of this specific failure mode rather than only handling its symptom.
4. **Separately, worth fixing at the ops/config layer**: this VM's `powercfg /query SCHEME_CURRENT SUB_SLEEP` showed AC ("plugged in") sleep-after = never — yet it slept anyway on an idle timer. A VM likely reports as running on DC (battery) power internally regardless of the host's actual power source, meaning the *DC* sleep timer (10 minutes on this machine) is the one that actually applies. `powercfg /change standby-timeout-dc 0` (in addition to the AC setting already checked) would prevent this specific trigger for future test VMs — not a code fix, but worth doing before the next unattended run.

## Open questions / not yet confirmed

- **Attempted a direct visual check via `vmrun captureScreen`; the result was inconclusive, not confirmatory.** The capture returned a fully black 2.3KB PNG. This does not cleanly confirm the "dead promoted window" hypothesis — the user reaches this VM via Parsec, and Parsec commonly takes over the display pipeline in a way that can leave VMware's own console capture blank regardless of what's actually on screen for the user. Recorded honestly as ambiguous evidence, not proof either way. A real answer needs either a screenshot taken through Parsec's own session, or the user's direct visual confirmation of what they're seeing.
- Still open: which of the four stacked windows (if any) is actually rendering correctly — the log evidence strongly implies at least one promoted window is dead, but this remains inference from logs + a `EnumWindows` dump, not a direct visual check.
- Why `srv-liveness` missed 2 consecutive probes right after resume, then apparently recovered on its own (no further missed-probe lines after `[1788483284]`) — likely just the backend catching up on deferred timer work after a long suspend, but not directly traced.
- Whether the `9ceeb40c` connection in the `ws egress lane full` burst is definitively the pre-sleep zombie connection, versus some other in-flight connection — inferred from conn-id mismatch with the freshly-connected `75390d27`, not confirmed by reading server-side connection-tracking state directly.
