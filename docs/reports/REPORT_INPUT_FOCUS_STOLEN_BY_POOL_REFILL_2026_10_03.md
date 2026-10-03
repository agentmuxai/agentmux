# Report: typing stops mid-sentence because a pool refill steals focus

**Date:** 2026-10-03
**Platform:** Windows
**Symptom (user report):** "Sometimes I am typing in the agent pane and, without any reason, the focus leaves the input box and typing no longer produces text. I then need to manually select the input to continue typing." The user suspected it coincided with a sound.

## Summary

When commit (RAM + page file) pressure returns to `Normal`, the host refills both warm pools at once (`memory_heartbeat.rs`). Each refill creates a top-level pool window and a pane-pool window. On Windows, each new pool window is **activated during its own creation** and becomes the **foreground window**, even though it is hidden or parked at (-32000, -32000). Keyboard focus goes with it: keystrokes land in a window the user cannot see. Clicking back into AgentMux fixes it, which is what the user did.

The sound correlation is real but indirect. Commit pressure drops when another agent's build finishes. A finished build usually ends that agent's turn, which plays its completion sound. Both happen within seconds of each other, and the focus steal comes with the pressure drop.

## Evidence

From the host's own focus and memory-pressure logging over 24 hours, on two running instances:

| Instance | Pool refills, last 24 h | Followed by the user clicking back into AgentMux within 60 s |
|---|---|---|
| A | 76 | 9 (five within 6 s) |
| B | 49 | 1 |

On instance A, commit pressure changed level 151 times in the day: 75 to warn, 12 to critical, 64 back to normal. Every return to normal was followed by both pool spawns and by two activations of a newly created pool window. The focus-restore observer only records an activation when a window is actually becoming active (`activation_state != WA_INACTIVE`).

The order of events in one refill:

1. Commit pressure returns to `Normal`; the pane pool and the window pool each spawn a window.
2. The new pane-pool window is activated about 20 ms later.
3. The new top-level pool window is activated **inside** `window_create_top_level`, before that call returns. Keyboard focus leaves the window that had it.
4. Windows reports the pool window as the new foreground window (`EVENT_SYSTEM_FOREGROUND`).
5. Four seconds later the main window is activated by a click (`WA_CLICKACTIVE`): the user clicking back in.

`on_load_end` already declines to show pool windows (`client/navigation.rs`, "pool windows skip the show/focus block entirely"), but that does not help: the activation happens earlier, inside `window_create_top_level`, and for the pane pool inside its Win32 popup's browser creation.

## Fix

1. **An invisible window never keeps activation** (`client/wndproc.rs`). The `WM_ACTIVATE` observer, already installed on every top-level window including pool windows, now checks whether the user can see the window being activated: visible, and on some monitor. If not, and the window it took activation from is one the user can see, it remembers that window and posts itself `WM_AGENTMUX_HAND_BACK_ACTIVATION`. When that message runs, after creation has finished, activation goes back to the remembered window, but only if the foreground window is still an invisible window of ours. That re-check matters because a pool window being promoted is activated while still hidden (`promote_pool_window` step 1) and then shown; by the time the posted message runs it is the visible foreground window the user asked for, and is left alone. Re-activating the main window runs its own `WM_ACTIVATE` restore, which re-focuses its render widget, so the caret is back where it was. A chain (one pool window activating after another) hands back to the first window remembered. Decision logic is the pure `invisible_activation`, unit-tested.
2. **Refill only after pressure settles** (`memory_pressure.rs` `PoolRefillGate`, `memory_heartbeat.rs`). The pools are refilled once commit pressure has stayed `Normal` for `POOL_REFILL_SETTLE` (3 minutes), not on every return to `Normal`. While builds run, pressure flaps; refilling on each flap created windows that were destroyed again minutes later, and each creation was a chance to steal focus. Eviction on entering Warn/Critical is unchanged.

Fix 1 alone stops the steal. Fix 2 removes most of the window churn that triggered it.

## Not changed

- Why Chromium/CEF activates a hidden top-level window during browser creation was not traced to a specific call. Fix 1 handles it wherever it comes from.
- The launcher's pool `DriftDetected { host_count: 1, mirror_count: 173 }` counter grows by one on every refill. That looks like a separate bookkeeping bug and is left for its own issue.
- macOS and Linux are untouched; the activation path is Windows-specific.
