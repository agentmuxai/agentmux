# RETRO — VM suspend leaves a "running but frontend-less" AgentMux instance; double-click on the exe recovers nothing visible

**Date:** 2026-09-03
**Author:** Camper
**Severity:** Medium — no data lost, the instance's backend stayed healthy throughout, but the user's only recovery action (double-click the exe) produced no working window. On a real user's machine (not an agent-operated VM), this reads as "AgentMux is broken, I have to kill it in Task Manager."
**Area:** `agentmux-launcher` single-instance forwarding (`other_instances.rs`), the `pool_respawn_on_promote` saga (`saga/pool_respawn.rs`), `agentmux-cef`'s `promote_pool_window` (`commands/window_pool.rs`), Windows/VM power management.
**Status:** historical — see Update 2026-09-04 below. The root cause originally documented in this retro is no longer believed to be what actually happened; the code-level observations remain accurate readings of the code but are not proven to be what caused the original symptom.

---

## Update 2026-09-04 — the real root cause was investigation tooling, not the app

**The VM-suspend/stale-pool-window narrative below is very likely wrong about what actually happened, though it's an honest record of what the evidence available at the time supported.** Discovered while trying to reproduce the "double-click does nothing" symptom again, live, with the repo owner watching their own screen at the same time as the investigation:

Every AgentMux instance launched via `vmrun runProgramInGuest` throughout this investigation — including the very first one, the one whose sleep/wake cycle this retro describes — ran in **Windows Session 0** (the non-interactive services session), not **Session 1** (the real interactive console session the repo owner actually sees via Parsec). Confirmed directly:

```
> query session
 SESSIONNAME               USERNAME                 ID  STATE   TYPE        DEVICE
>services                                            0  Disc
 console                   Flor                      1  Active

agentmux pid=11052 SessionId=0
explorer pid=5596 SessionId=1
winlogon pid=732 SessionId=1
```

This is a known VMware/VIX limitation, not an AgentMux bug: guest automation via `vmrun` runs through the VMware Tools guest service, and processes it spawns land in Session 0 regardless of the `-activeWindow` flag or real guest credentials (`-gu`/`-gp`) passed to it. Session 0 has had no user-visible desktop since Windows Vista's session-0-isolation change — nothing running there can ever paint a window a real user sees, no matter how correctly the app itself behaves.

AgentMux's single-instance enforcement uses a named pipe, which **is** global across sessions in Windows. So once a Session-0 instance existed and held that pipe, every subsequent launch attempt — including the repo owner's own genuine double-clicks, from their real Session 1 desktop — correctly detected "already running" and forwarded an `open_new_window` request to the Session-0 instance, which correctly ran its promote/refill saga and correctly connected a new WebSocket, all while being structurally incapable of ever showing the result to anyone. That's why every log signal this retro's original investigation checked looked healthy (clean disconnect handling, working reconnect logic, a real Windows `IsWindow()` liveness check, a fast forward, a completed saga, a fast WebSocket setup) while the user's screen showed nothing at all.

**Resolution:** killed the Session-0 instance (`killProcessInGuest`), confirmed zero AgentMux processes remained anywhere on the VM, then had the repo owner double-click the exe themselves from their own session. It opened correctly on the first try.

**What this changes about the analysis above:**

- The specific claim that a VM sleep/wake cycle left a promoted pool window carrying a dead connection is **no longer supported** — the instance being investigated was never in a session where anyone could observe whether that was true, and the actual explanation for "nothing visible happens" (a Session-0 zombie holding the single-instance pipe) doesn't require the sleep/wake cycle at all. A single `vmrun`-launched instance sitting untouched would have produced the exact same "double-click does nothing" symptom with no sleep involved.
- **What still stands, as a general observation about the code, independent of what caused this specific incident:** `promote_pool_window`'s Windows-specific liveness check (`IsWindow()` against a cached HWND) validates that the OS hasn't destroyed a window handle — it does not and cannot validate that the *session* holding that handle is one any user can observe, or that the renderer behind it is responsive. That's a real, accurate reading of the code. It just isn't what happened here.
- **The recommendation to verify (not just dispatch) before considering `open_new_window` satisfied is downgraded from "fixes a confirmed incident" to "reasonable additional hardening, motivated by a code-level gap rather than by this specific incident."** Not retracted, but should not be cited as proof this failure mode occurs in normal (non-agent-automated) use.
- The ops-layer power-timer finding (VM's DC/battery sleep timer firing despite the AC timer showing "never") is unaffected by this correction and remains accurate as recorded — it was independently confirmed by direct `powercfg` inspection, twice, and has since been fixed on this VM (disabled on both AC and DC).

**Lesson for future agent-driven VM testing, recorded so this doesn't repeat:** don't use `vmrun`/VIX guest automation to launch or interact with anything meant to be visible to a human on that VM. It's fine for file transfer, running scripts, and querying state, but any GUI process it spawns lands in a session the human can never see. For anything meant to be seen or clicked, have the human do it, or restrict agent involvement to read-only inspection (logs, `query session`, process listing) rather than driving the app directly.

## Summary

While testing the fresh-PC onboarding fix (#2942/#2943/#2947) on a real Windows 11 VM, the VM was left running unattended and went to sleep for ~4h15m (`Sleep Reason: System Idle`, confirmed via the Windows System event log). The running AgentMux instance's WebSocket connection dropped at the moment sleep began — **and, corrected from an earlier draft of this retro (Codex review, PR #2957), the backend actually did detect and cleanly handle that disconnect** (`agentmux-srv/src/server/websocket.rs`'s `handle_ws_connection` logs `"WebSocket client disconnected"` and runs `unregister_ws`/`unsubscribe_all` right after its read loop exits — that's a real, logged detection, not a silent failure), and the frontend has its own automatic reconnect logic (`frontend/app/store/ws.ts`'s `onclose` → `reconnect()`, backoff-timed, up to 20 attempts). What's genuinely unproven, not what was originally claimed here, is *why* that reconnect machinery didn't restore a working connection across a multi-hour suspend — see Root Cause below. When the user later double-clicked the exe expecting a fresh window, the launcher did exactly what it's designed to do — detected the still-running instance via the named pipe and forwarded an `open_new_window` request — but the window that request produced was **a pre-warmed "pool" window promoted from before the sleep**, and the user saw nothing respond. Four overlapping top-level AgentMux windows now exist on screen at conflicting positions from repeated attempts, none of them confirmed working.

## Timeline (all times reconstructed from real logs — `agentmux-launcher.log` — and the Windows System event log, not inferred)

- **~12:53 local** — first clean launch of the app on this VM (separate, unrelated investigation of first-launch latency — see tracking issue #2940).
- **13:38:59** — `Microsoft-Windows-Kernel-Power` Event ID 42: *"The system is entering sleep. Sleep Reason: System Idle."*
- **20:38:57.020135Z** (`agentmux-launcher.log`) — `WebSocket client disconnected conn_id=b7f3c2a9-...`. This is ~1 second after the sleep event above (UTC/local offset accounts for the hour gap between the two timestamps). The disconnect itself is real and was properly detected — the backend's read loop exited (network state torn down by suspend, not a graceful close frame from the client) and the server ran its normal cleanup (`unregister_ws`/`unsubscribe_all`). What's not established is whether this specific `b7f3c2a9` connection belonged to one of the pool windows later promoted, or to an unrelated already-open window — not traced.
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

**Corrected (Codex review, PR #2957 round 1):** an earlier draft of this retro quoted the wrong platform's code for the promote path and mischaracterized the disconnect as unnoticed. Neither survives scrutiny as originally written. The actual picture, verified directly against the Windows-specific code (this incident happened on Windows; the non-Windows fallback in the same file is irrelevant here):

```rust
// agentmux-cef/src/commands/window_pool.rs:1009 — #[cfg(target_os = "windows")]
pub fn promote_pool_window(...) -> Option<String> {
    ...
    let raw_hwnd: Option<*mut std::ffi::c_void> = match state.get_browser(&label) {
        None => { /* log + None */ }
        Some(browser) => match browser.host() {
            None => { /* log + None */ }
            Some(host) => {
                let cef_hwnd = host.window_handle().0;
                if !cef_hwnd.is_null() {
                    Some(cef_hwnd as *mut std::ffi::c_void)
                } else {
                    // CEF lost the reference — fall back to cache, then
                    // verify the cached HWND is still a live OS window.
                    let cached = pool_hwnd_cache().lock().unwrap().get(&label).copied();
                    match cached {
                        None => None,
                        Some(h) => {
                            let alive = unsafe { IsWindow(h as HWND) } != 0;
                            if alive { Some(h as *mut std::ffi::c_void) } else { None }
                        }
                    }
                }
            }
        },
    };
```

This is a genuinely more thorough check than "does an object exist in a map" — it resolves the browser, its host, the CEF-reported HWND (falling back to a cache, a known CEF quirk per `SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md`), and validates that cached handle against the real OS window table via Win32 `IsWindow()`. **It is still insufficient for this specific failure mode, but for a narrower and more precise reason than the earlier draft claimed**: `IsWindow()` only confirms the OS hasn't destroyed or recycled the window handle. A VM suspend/resume cycle does not destroy window handles — nothing in the OS reclaims them just because the machine slept — so `IsWindow()` returns true regardless of whether the CEF renderer process behind that handle is still responsive or whether its page's WebSocket connection survived. The check validates window-handle existence, not renderer or connection health, and none of the Windows promote path's checks reach either of those.

**What is NOT established, and should not be presented as settled:** whether the specific promoted pool window(s) actually carried a dead connection. The backend did detect and cleanly handle at least one disconnect (`b7f3c2a9`, see Timeline), and the frontend's `ws.ts` has automatic reconnect with up to 20 backoff-timed attempts — this is not a system with no recovery path. A plausible mechanism for why recovery still failed: browser/renderer `setTimeout` scheduling does not fire while a process is suspended, so a reconnect timer armed before sleep would not "count down" during the 4h16m gap — but if `reconnectTimes` was already elevated before sleep, or the scheduling resumes in a way that immediately exhausts the remaining attempts against a still-unreachable backend right at wake, the 20-attempt budget could burn out in the first few seconds after resume, before the backend's own `srv-liveness` had recovered from its own missed probes (see Timeline). This is a plausible, mechanism-grounded hypothesis, not a confirmed one — it has not been traced through actual `reconnectTimes` values or `ws.ts` log output from this incident.

The saga (`pool_respawn_on_promote`) itself doesn't help either way — it only tracks the pool *refill* (spawning a replacement background window), not whether the just-promoted window is functional. Its own doc comment (`saga/pool_respawn.rs:70-79`) states the failure posture plainly: *"if refill genuinely fails, the next promote will start a fresh saga"* — there is no concept anywhere in this path of "the promote itself produced a window that isn't actually usable," independent of whatever the real mechanism turns out to be.

## Why the user saw "nothing happens"

The launcher did not fail silently — it correctly forwarded the request and the host correctly ran its promote/refill machinery, logging success the whole way, and every layer involved (backend disconnect handling, frontend reconnect, Windows HWND validation) has real, working failure-handling of its own. The gap is that none of those individually-correct mechanisms compose into an end-to-end guarantee: the promote path confirms a window *handle* is valid, not that the page behind it is connected and responsive, and whatever actually broke the frontend connection (most plausibly, but not confirmed, an exhausted reconnect budget across a multi-hour suspend) sits below the level any of these checks look at. From the user's side, double-clicking the exe either re-surfaced an already-broken window, or added a new one to the stack in the same state — visually indistinguishable from "nothing happened."

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
- **New, from the Codex-prompted correction above**: whether the promoted windows' frontends actually failed to reconnect, and if so, by which mechanism — the "reconnect budget exhausted across a multi-hour suspend" explanation is plausible and grounded in real code (`ws.ts`'s 20-attempt cap, browser timer suspension), but not traced through actual reconnect-attempt logs from this incident. This is now the load-bearing open question the original draft treated as settled.
- Whether `b7f3c2a9` (the one disconnect actually observed in the launcher log) belonged to any of the windows later promoted, or to some other already-open window entirely unrelated to the double-click's outcome.
