# SPEC: Dead Terminal Pane — Silent PTY Failure & RT Info Race

**Date:** 2026-05-20  
**Status:** Open — pending fix  
**Observed on:** v0.34.0 (Windows 11)  
**Log evidence:** `~/.agentmux/versions/0.34.0/logs/agentmux-host-v0.34.0.log.2026-05-20`

---

## Summary

Opening a new terminal pane sometimes produces a "dead" pane: xterm.js renders a
blinking cursor, but no shell prompt appears and keystrokes do nothing. Two distinct
root causes were identified from logs. Both produce identical user-facing symptoms.

---

## Observed Symptoms

- Terminal pane opens with a blinking cursor, black background.
- Pressing any key has no effect.
- Host log floods with `[fe] lastHandledEvent return false` (~6 Hz, one per keypress).
- No error message shown inside the pane or in any UI surface.

---

## Root Cause 1 — Silent PTY Spawn Failure Under Memory Pressure

### Evidence

When `load_pct ≥ 93` (observed: `avail_phys_gb: 4.2 / 61.6 GB`):

- `object.CreateBlock` succeeds in the backend.
- **No** `resync_controller`, `PTY opened`, or `process spawned` entries appear in the
  sidecar log for the new block.
- xterm.js still mounts (it needs no process), showing the blinking cursor.
- The block sits in the layout with xterm running but no PTY behind it.

### What Happens

`portable-pty` / the OS shell spawn fails because there is insufficient memory to
fork a new process. The failure is either swallowed silently in the block controller
or the controller is never invoked. No error propagates to the frontend.

### Missing Signals

1. No `ERROR`-level log entry in sidecar when shell spawn fails.
2. No error event pushed to the block's `blockfile` stream.
3. No memory-pressure warning in any UI surface before or after the failure.

---

## Root Cause 2 — RT Info Race on Pane Open

### Evidence

Even with sufficient memory (`load_pct: 52`, `avail_phys_gb: 29.4 GB`), block
`c88fc5f9` was opened and:

```
[srv]  PTY opened  rows=25 cols=80
[srv]  process spawned successfully
[fe]   New BlockAtom c88fc5f9 changeConn
[fe]   object.GetObject block:c88fc5f9 | 64ms          ← slow fetch
[fe]   long-task 51ms
[fe]   long-task 50ms
[fe]   [reactive] registered agent Terminal -> c88fc5f9
[fe]   error setting RT info {}                         ← RT info call failed
[fe]   setFocusedChild c88fc5f9 ... textarea.xterm-helper-textarea
[fe]   lastHandledEvent return false  (repeated every ~170ms)
```

The PTY is alive at the default 25×80 but the frontend's `setRTInfo` call returned
`{}` (empty / error response). Without RT info the shell may not emit a prompt and
the `blockinput` path appears unresponsive to the frontend.

### What Happens

There is a race between:
1. The renderer performing two 50 ms long-tasks (layout + WaveObj resolution) that
   block the JS thread.
2. The reactive `Terminal` agent registering and immediately calling `setRTInfo`.

By the time `setRTInfo` fires, some required state (the WS subscription, the block
controller's reactive channel, or the block's runtime-info slot) is not yet ready.
The service returns `{}` instead of a success response. The frontend swallows this
as a non-fatal path and proceeds, leaving the PTY at the wrong size and the
frontend's WS message pump in an undetermined state.

---

## Memory Pressure Timeline (from `mem_heartbeat`)

| Time     | `load_pct` | `avail_phys_gb` | Effect                            |
|----------|-----------|-----------------|-----------------------------------|
| 10:15:xx | 93%       | 4.2             | First dead pane — PTY never spawned |
| 10:16:32 | 72%       | 17.2            | User freed memory                 |
| 10:16:52 | 56%       | 27.1            | Stabilising                       |
| 10:21:xx | 52%       | 29.4            | Second dead pane — RT info race   |

The memory threshold at which shell spawn fails is not precisely known. It likely
depends on commit charge, not just physical pages.

---

## Proposed Fixes

### Fix A — Backend PTY Spawn Error Propagation (Root Cause 1)

In `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs`, when the shell process
spawn fails:

1. Log at `ERROR` level with the OS error code.
2. Push an error record onto the block's `blockfile` output stream immediately, so
   the frontend receives it via its existing `blockfile` event subscription.
3. Set a `pty_failed: true` flag on the block metadata so any reconnecting frontend
   can detect the failure without waiting for another event.

The error record written to `blockfile` uses a distinguished prefix so the frontend
can detect it without ambiguity (e.g. `\x1b[31mShell failed to start: <os error>\x1b[0m\r\n`
rendered directly by xterm.js, plus a structured metadata flag).

### Fix B — Frontend Error Overlay (reacts to Fix A, Root Cause 1)

When the frontend's `blockfile` event handler receives the structured error record,
it renders an error overlay in place of the cursor:

```
┌─────────────────────────────────────────────────────┐
│  Shell did not start                                │
│                                                     │
│  The terminal process failed to launch.             │
│  This is often caused by low system memory.         │
│                                                     │
│  [Retry]   [Close]                                  │
└─────────────────────────────────────────────────────┘
```

No timeout or polling needed — the overlay appears as soon as the backend error
event arrives. **Retry** calls `block.restart`; **Close** calls `object.DeleteBlock`.

Touch-points:
- `frontend/app/view/term/termwrap.ts` — handle the `pty_failed` metadata flag in
  the `blockfile` event; expose a `ptyFailed` signal to the view.
- `frontend/app/view/term/term.tsx` (or equivalent) — render the overlay when
  `ptyFailed === true`.

### Fix C — RT Info Retry / Defer (Root Cause 2)

In the frontend terminal init path, `setRTInfo` should not be fire-and-forget. If
it returns `{}` or an error:

1. Retry with exponential backoff (3 retries: 100 / 300 / 1000 ms).
2. If all retries fail, surface the same error overlay as Fix B.

The long-tasks (51 ms + 50 ms) that precede the race should also be investigated.
They occur during `WaveObj` resolution and layout update on the new block — if they
can be split or deferred, the RT info call arrives at a more settled state.

### Fix D — Memory Pressure Warning (pre-emptive UX)

The `mem_heartbeat` target already fires every 20 s. Expose this to the frontend:

| `load_pct` | Action |
|-----------|--------|
| ≥ 85%    | Show amber warning badge in the title bar / status strip |
| ≥ 95%    | Show red badge + toast: "System memory critical — terminal panes may fail to open" |

Implementation: the host already logs `mem_heartbeat` — add a CEF → frontend IPC
message (or include the latest heartbeat in the `GetSysInfo` poll response) so the
renderer can display the badge without an additional RPC.

---

## Investigation Notes

- **`[fe] lastHandledEvent return false`**: Logged when `xterm.js` receives a key
  event but the upstream handler (the `blockinput` WS sender) declines to process
  it. This is a symptom, not a cause; the cause is either missing PTY (Root Cause 1)
  or the broken RT info path (Root Cause 2).

- **Block `67c5580d`** (original dead pane from the 93% session): still in the
  layout tree as of the log snapshot. Its controller was never started so the block
  is permanently inert. It should be garbage-collected or the user should be able
  to retry it.

- **`failed to watch directory for subagents: ~/.config/claude-terminal`** (logged
  on every terminal open): non-fatal but noisy. The path `~/.config/claude-terminal`
  does not exist. Either create it on first use or make the watcher optional.

---

## Related

- `docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` — PTY echo-latency
  benchmark; the "shell never started" case produces ∞ latency.
- `frontend/app/view/term/termwrap.ts` — xterm.js wrapper; the `writeInFlight`
  fast-path fix (PR #926) is in this file.
- `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` — PTY spawn site.
