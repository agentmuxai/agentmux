# Report: typing stops mid-sentence because a pool refill steals focus

**Date:** 2026-10-03
**Platform:** Windows
**Symptom (user report):** "Sometimes I am typing in the agent pane and, without any reason, the focus leaves the input box and typing no longer produces text. I then need to manually select the input to continue typing." The user suspected it coincided with a sound.

## Summary

When commit (RAM + page file) pressure returns to `Normal`, the host refills both warm pools at once (`memory_heartbeat.rs`). Each refill creates a top-level pool window and a pane-pool window. On Windows, each new pool window is **activated during its own creation** and becomes the **foreground window**, even though it is hidden or parked at (-32000, -32000). Keyboard focus goes with it: keystrokes land in a window the user cannot see. Clicking back into AgentMux fixes it, which is what the user did.

The sound correlation is real but indirect. Commit pressure drops when another agent's build finishes. A finished build usually ends that agent's turn, which plays its completion sound. Both happen within seconds of each other, and the focus steal comes with the pressure drop.

## Evidence

From `agentmux-host-v0.59.4.log.2026-10-03` (and the 10-02 file, plus a v0.59.7 instance):

| Instance | Pool refills, last 24 h | Followed by the user clicking back into AgentMux within 60 s (`WM_ACTIVATE state=2`) |
|---|---|---|
| v0.59.4 | 76 | 9 (five within 6 s) |
| v0.59.7 | 49 | 1 |

Commit pressure changed level 151 times in the day on v0.59.4: 75 to warn, 12 to critical, 64 back to normal. Every return to normal was followed by the `[pane-pool] spawning` and `[pool] spawning` lines and by two `[focus-restore] WM_ACTIVATE root=<new pool window> no recorded child` lines. That log line is only written when a window is being activated (`activation_state != WA_INACTIVE`).

One refill, 2026-10-03 19:31 UTC:

```
19:31:23.334 mem_pressure  page file (commit) pressure changed      (to normal)
19:31:23.335 pool:pane     [pane-pool] spawning pane pool window
19:31:23.355 wndproc       [focus-restore] WM_ACTIVATE root=0x49115e (floating-pool-…) no recorded child
19:31:23.755 wndproc       [focus-restore] installed WM_ACTIVATE observer on 0x2930b12 (window-pool-…)
19:31:23.761 wndproc       [focus-restore] WM_ACTIVATE root=0x2930b12 no recorded child
19:31:23.761 pane-wndproc  WM_KILLFOCUS hwnd=0x16c0ea2            (focus leaving the previous pool window)
19:31:23.764 create-window window_create_top_level returned        (activation happened inside creation)
19:31:23.768 wrr           callback event=0x3 hwnd=0x2930b12        (EVENT_SYSTEM_FOREGROUND: pool window is foreground)
19:31:27.398 wndproc       [focus-restore] WM_ACTIVATE root=0x30c6c (main) state=2   (user clicks back, 4 s later)
```

`on_load_end` already declines to show pool windows (`client/navigation.rs`, "pool windows skip the show/focus block entirely"), but that does not help: the activation happens earlier, inside `window_create_top_level`, and for the pane pool inside its Win32 popup's browser creation.

## Fix

1. **An invisible window never keeps activation** (`client/wndproc.rs`). The `WM_ACTIVATE` observer, already installed on every top-level window including pool windows, now checks whether the user can see the window being activated: visible, and on some monitor. If not, and the window it took activation from is one the user can see, it remembers that window and posts itself `WM_AGENTMUX_HAND_BACK_ACTIVATION`. When that message runs, after creation has finished, activation goes back to the remembered window, but only if the foreground window is still an invisible window of ours. That re-check matters because a pool window being promoted is activated while still hidden (`promote_pool_window` step 1) and then shown; by the time the posted message runs it is the visible foreground window the user asked for, and is left alone. Re-activating the main window runs its own `WM_ACTIVATE` restore, which re-focuses its render widget, so the caret is back where it was. A chain (one pool window activating after another) hands back to the first window remembered. Decision logic is the pure `invisible_activation`, unit-tested.
2. **Refill only after pressure settles** (`memory_pressure.rs` `PoolRefillGate`, `memory_heartbeat.rs`). The pools are refilled once commit pressure has stayed `Normal` for `POOL_REFILL_SETTLE` (3 minutes), not on every return to `Normal`. While builds run, pressure flaps; refilling on each flap created windows that were destroyed again minutes later, and each creation was a chance to steal focus. Eviction on entering Warn/Critical is unchanged.

Fix 1 alone stops the steal. Fix 2 removes most of the window churn that triggered it.

## Not changed

- Why Chromium/CEF activates a hidden top-level window during browser creation was not traced to a specific call. Fix 1 handles it wherever it comes from.
- The launcher's pool `DriftDetected { host_count: 1, mirror_count: 173 }` counter grows by one on every refill. That looks like a separate bookkeeping bug and is left for its own issue.
- macOS and Linux are untouched; the activation path is Windows-specific.
