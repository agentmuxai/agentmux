# SPEC — Detect a hung app renderer nobody is interacting with

**Status:** Draft — plan under verification (see §6). Live tests are delegated to an isolated test box: `docs/plans/PLAN_IDLE_HUNG_RENDERER_DETECTION_TEST_2026_09_24.md`
**Date:** 2026-09-23
**Author:** Agent3
**Parent:** `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` §8.1 (remaining gap), handoff `docs/status/STATUS_RENDERER_DEADLOCK_AND_CEF_RUNTIME_R2_HANDOFF_2026_09_23.md` §5.2
**Builds on:** #3594 (`agentmux-cef/src/client/unresponsive.rs`)

## 1. Problem

#3594 recovers a hung app-UI renderer, but only if Chromium's hang monitor
reports it, and Chromium only measures while an input event is waiting for the
renderer to acknowledge it. A window that freezes while nobody is clicking or
typing in it is never reported: it stays frozen until someone touches it, and
then takes a further ~30 s. The 2026-09-22 freeze sat unnoticed like this.

## 2. Idea

Don't build a second kill path. Keep the kill decision where it is (Chromium's
input-ack measurement + #3594's two-strike rule) and add only a cheap
**liveness probe** that, when it goes quiet, **arms** that detector by
sending the window one harmless input event.

```
host, every 10 s, each app-UI browser:
  execute_java_script("console.debug('<sentinel> <seq>')")
  on_console_message(sentinel) → last_alive[browser] = now   (return 1: swallow it)

if now - last_alive[browser] > 25 s  (≥2 missed probes):
  send_key_event(browser, F24 key-up)        ← one "poke", logged
  → renderer can't ack it → Chromium reports after ~15 s
  → #3594: wait(), then terminate() at ~30 s → recovery page
```

Why this shape:

- **False positives are nearly free.** If the probe misfires but the renderer
  is alive, it simply acknowledges the poke and nothing happens. A healthy app
  sees one F24 key-up, which nothing in the frontend handles.
- **No new kill logic, no renderer PID lookup** (CEF exposes neither a
  terminate API outside the callback nor a browser→PID map).
- **Exact identity.** `on_console_message` hands back the very `Browser` that
  answered, so no window-label/OID mapping is needed.

## 3. Rejected alternatives

| Option | Why not |
|---|---|
| srv-side "N s of `ws egress lane full`" (the handoff's first idea) | srv can't tell which window a WS connection belongs to (`tab_id` is always empty; `?tabid=` is ignored). The lane only fills when there is enough traffic, so on an idle app it takes minutes. srv→host is only HTTP to the host's IPC port. |
| srv app-level ping (srv already pings every WS every 10 s and the frontend JS answers `pong`, which srv ignores) | Same identity problem. The ping `send().await` sits inside the connection loop, so a jammed socket blocks the loop that would notice. |
| CEF process messages (renderer-side `RenderProcessHandler`) | Most exact, but adds the first renderer-process code path. `ipc.rs` deliberately avoided this. Keep it as an upgrade if the console sentinel proves noisy. |
| Host sends input continuously to every window | Invasive: IME composition, user activation, and key state on every window forever. The poke is sent only after the probe has already gone quiet. |
| Kill directly when the probe fails | No API to do it, and it would drop the "two independent signals" rule (§10-C of the parent spec). |

## 4. Scope

- **In:** `TopLevel` (including pool windows) and `Floater` browsers, the same set #3594 covers.
- **Out:** browser panes, auth popups, and DevTools, same as #3594.
- **Minimized/hidden windows:** the probe still runs, but Chromium does not
  run the hang monitor for a hidden widget, so recovery happens when the
  window is shown again (§6, V5). A hidden frozen window costs nothing visible
  until then.

## 5. Implementation sketch

- `agentmux-cef/src/client/liveness.rs` (new):
  - The probe tick is a `post_delayed_task` on the UI thread every 10 s.
    CEF calls must happen on the UI thread.
  - `last_alive: HashMap<i32 /*browser id*/, Instant>`, plus `poked_at` so there
    is at most one poke per hang episode. The poke is re-armed when the browser
    answers again or is recreated.
  - Logs at the `crash` target: `renderer_probe_silent` (the poke), and
    `renderer_probe_alive_again` after a poke.
- `on_console_message` in `handlers.rs`: match the sentinel prefix, record
  liveness, and return 1. All other messages are unchanged.
- Entries are dropped in `on_before_close`, as the other per-browser maps are.
- Thresholds are constants: 10 s probe, 25 s silence. Expected worst case to
  recovery page ≈ 25 + 30 = 55 s with no interaction (today: never).
- No frontend, srv or settings changes.

## 6. Verification plan (assumptions to prove before implementing)

| # | Assumption | How |
|---|---|---|
| V1 | `execute_java_script` + console sentinel round-trips through `on_console_message` with the right `Browser`, and returning 1 keeps it out of `cef-debug.log` | spike + log inspection |
| V2 | A hung renderer produces no sentinel, and `execute_java_script` into it does not block the host UI thread | spike, main-thread hang |
| V3 | **Crux:** a host `send_key_event` (F24 key-up) into a hung, visible, **unfocused** window arms Chromium's hang monitor → #3594 kills it, with no OS input and no debugger attached | spike, dev build |
| V4 | The poke is inert on a healthy window (no visible effect, no terminal input) | spike + code read |
| V5 | Minimized window: no kill while minimized; restoring it gets it recovered | spike |
| V6 | The probe's cost is negligible (one tiny script per window per 10 s) | log/CPU observation |

A compositor-thread deadlock (the 09-22 case) cannot be reproduced on demand.
It is covered by reasoning: key events are always routed through the
compositor thread, so in that deadlock the poke is never acknowledged either.
