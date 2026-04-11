# Window-Process Coupling Analysis

**Date:** 2026-04-05

---

## Background

AgentMux is a CEF-based desktop terminal app. The process tree looks like:

```
agentmux.exe (launcher, windows_subsystem="windows", no visible window)
  └─ agentmux-cef-0.33.46.exe (CEF browser host — owns windows → appears in Task Manager "Apps")
       ├─ agentmux-cef-0.33.46.exe --type=gpu-process
       ├─ agentmux-cef-0.33.46.exe --type=renderer (per window)
       └─ agentmux-srv-0.33.46-windows.x64.exe (backend, Job Object KILL_ON_JOB_CLOSE)
            ├─ pwsh.exe (terminal panes, children of agentmux-srv)
            └─ pwsh.exe
```

Task Manager groups processes by parent PID. Shells appear under `agentmux-srv` which appears under `agentmux-cef`. This grouping is only maintained while `agentmux-cef` has a visible window — otherwise everything falls into "Background processes."

---

## The Two Requirements That Keep Conflicting

**Requirement A (Shell Cleanup):** When a secondary window closes, all shell processes (pwsh, bash, cmd) for that window's tabs/panes must exit. Without cleanup, they become resource-wasting orphans.

**Requirement B (Task Manager Grouping):** Shell processes must remain visually grouped under `agentmux-cef` in Task Manager while alive. They must not appear as independent "Background processes."

---

## Why These Requirements Conflict

Task Manager grouping is based on **live process tree** (parent-child PID relationships). A process appears "independent" in Background processes if:

1. It has no parent with a visible window, OR
2. It died while its parent was still alive (the act of dying is seen as "detaching")

Both requirements involve the same event: a secondary window close. The conflict is about **when** shells die relative to the CEF process being alive:

- **Kill shells while CEF is alive** → they die independently → briefly appear as ungrouped before disappearing from Task Manager
- **Kill shells after CEF exits** → too late for secondary window close (CEF only fully exits when ALL windows close via Job Object)

The true fix requires killing shells in a window narrow enough that Task Manager does not register them as independent: either before the window is visible (impossible) or after the CEF renderer for that window is already gone.

---

## Three Attempts and Why Each Failed

### Attempt 1: No Cleanup (before PR #299, through v0.33.44)

No code to kill shells on window close.

**Result:**
- Shell cleanup: shells stay alive as orphans indefinitely
- Grouping: shells stay alive and remain grouped under `agentmux-cef`

### Attempt 2: PR #299 + `beforeunload` Handler (v0.33.45)

Code added to `frontend/wave.ts`:

```typescript
window.addEventListener("beforeunload", () => {
    const wid = windowId();
    if (wid && windowCountAtom() > 1) {
        WindowService.CloseWindow(wid).catch(() => {});
    }
});
```

`beforeunload` fires before the CEF window is destroyed. The `fetch()` to `agentmux-srv` goes out, and the backend kills shells via `delete_controller()`. The shells die while `agentmux-cef.exe` and `agentmux-srv.exe` are still running.

**Additional bug:** `windowCountAtom` was a SolidJS signal set via `setWindowCountAtom`. When a secondary window closed, the CEF host emitted `"window-instances-changed"` only on OPEN (in `open_new_window`), never on close. So the main window's `windowCountAtom` stayed at 2 even after the secondary closed. This caused the main window's `beforeunload` to also call `CloseWindow` unnecessarily.

**Result:**
- Shell cleanup: shells die when window closes (`fetch` completes because `beforeunload` has a brief synchronous window in Chromium)
- Grouping: shells die while CEF is alive → they appear as independent background processes momentarily before disappearing

**User observation:** "i see the processes exit now, but before they were listed under the main CEF process, now they are independent in the background processes"

### Attempt 3: `on_before_close` Rust Hook (v0.33.46)

**Design:**

1. Frontend calls `registerBackendWindow(label, windowId)` IPC after `initWave()` completes, storing `window_label → backend_window_id` in `AppState.window_id_map`
2. CEF Rust `on_before_close` hook: looks up the closing browser's label → looks up backend window ID → spawns thread → raw TCP call to `agentmux-srv /wave/service?method=CloseWindow`
3. Also emits `"window-instances-changed"` to remaining windows (fixes the `windowCountAtom` stale count bug from Attempt 2)

**Result:**
- Grouping: fixed (user confirmed "i see they all stay together now")
- Tab close: fixed (`delete_tab_inner` calls `delete_controller` from PR #299)
- Shell cleanup on window close: broken again — shells do not exit when a secondary window closes

---

## Root Cause Analysis: v0.33.46 Window Close Failure

The `on_before_close` Rust hook spawns a thread that calls `backend_close_window()`. This function makes a raw TCP connection to `agentmux-srv` and sends an HTTP POST to `/wave/service`.

Three possible failure points have been identified.

### Failure Point A: `window_id_map` Is Empty for the Closing Browser

`backend_window_id` is looked up via:

```rust
self.state.window_id_map.lock().remove(lbl)
```

Where `lbl` comes from:

```rust
browsers.iter()
    .find(|(_, b)| b.is_same(Some(&mut browser)) != 0)
    .map(|(k, _)| k.clone())
```

If `b.is_same()` returns 0 for all browsers (identity comparison broken across CEF Rust binding clones), `label` = None → `backend_window_id` = None → only a warning is logged, no cleanup.

Alternatively, if the `register_backend_window` IPC call failed silently (`.catch(() => {})`), the entry was never added to `window_id_map`.

### Failure Point B: TCP Call Reaches Backend but Wrong Request Format

The raw HTTP body sent is:

```json
{"service":"window","method":"CloseWindow","args":["<window-id>"],"uicontext":null}
```

`agentmux-srv`'s `handle_service` at `server/service.rs:18` does `serde_json::from_slice(&body)` into `WebCallType`. Then `dispatch_service` does `service::get_arg(args, 0)` for the window ID, reading `args[0]` as the window UUID string. Deserialization as `String` should succeed.

Auth: the raw TCP request includes `?authkey={auth_key}` which matches what `auth_middleware` checks. This path is not the failure point.

### Failure Point C: Thread Timing

This is not the failure point. Axum processes the request before writing the response. The server-side processing (killing shells) happens regardless of whether the client reads the response.

---

## The Fundamental Design Tension

**CEF's process management model:** CEF is designed around browsers. Browser lifecycle events (`on_before_close`, `do_close`) are the canonical hooks for cleanup. But these hooks run while the CEF process is still alive, meaning any shells killed here die independently (not via CEF process exit), which is visible in Task Manager.

**Windows Task Manager's grouping model:** Task Manager groups by live parent-child PID relationships. Once a process dies, it's gone. If it dies while its parent chain is alive, there is a brief moment where Task Manager shows it as "detaching." With rapid kills (< 50ms from window close to shell exit), this flash is imperceptible. With slower kills (> 500ms), the user may briefly see shells in "Background processes" before they disappear.

**The actual conflict is timing, not architecture:**

- `beforeunload` kills shells while the window is still open (still showing on screen) → 500–2000ms while visible → noticeable ungrouped flash
- `on_before_close` Rust hook fires AFTER the window begins destroying → the shell kill happens after the window is visually gone → the ungrouped flash is imperceptible or zero

The `on_before_close` Rust approach is architecturally correct. The grouping problem with `beforeunload` arose because kills happened while the window was still visible. The v0.33.46 approach kills shells after the window is already gone — the issue is only that the lookup or TCP call is silently failing.

---

## Diagnosis Steps for v0.33.46

### Step 1: Add Visible Logging

In `client.rs` `on_before_close`, add an explicit log for the window ID lookup result:

```rust
tracing::info!(
    "[on_before_close] secondary window close: label={:?} backend_window_id={:?}",
    label,
    backend_window_id
);
```

### Step 2: Verify `register_backend_window` Is Being Called

In `commands/window.rs` `register_backend_window`, check CEF log for lines like:

```
[window] registered backend window ID label=window-{uuid} window_id={uuid}
```

If these lines are absent, the IPC call is not completing and `window_id_map` is never populated.

### Step 3: Verify Browser Identity Comparison

Add a debug log inside the `find` closure to count how many browsers are checked and whether any match. This will confirm whether `is_same` is returning 0 for all entries.

### Step 4: Alternative — Use Browser Identifier Directly

Instead of using `is_same` to find the browser label (which requires cloning the CEF browser object across the binding layer), store a `browser_id → label` map in `AppState` and populate it in `on_after_created`. CEF browsers have a unique integer ID accessible via `browser.identifier()`. This avoids the `is_same` cloning issue entirely:

```rust
// in on_after_created:
let id = browser.identifier();
self.state.browser_label_map.lock().insert(id, label);

// in on_before_close:
let id = browser.identifier();
let label = self.state.browser_label_map.lock().remove(&id);
```

---

## Definitive Solution

The definitive solution that achieves both requirements simultaneously:

1. **Keep the v0.33.46 Rust `on_before_close` approach.** This is architecturally correct. Shells killed here die after the window is visually gone, so Task Manager never registers them as independent background processes.

2. **Fix the `window_id_map` lookup by keying on `browser.identifier()` (integer ID) instead of label string.** This eliminates ambiguity from `is_same` comparisons across cloned CEF Rust binding objects.

3. **Keep `register_backend_window` but map by browser ID.** Have the frontend register `(windowLabel, backendWindowId)`. In Rust, map `browser_id → (label, backendWindowId)`, populated in `on_after_created` where the browser object is first available and unambiguous.

**Expected outcome:** Shells die after the window is visually gone. Task Manager never shows them as independent. Grouping is preserved until the moment of death. Both Requirement A and Requirement B are satisfied.

---

## Summary Table

| Version | Mechanism | Shell Cleanup | Task Manager Grouping |
|---|---|---|---|
| v0.33.44 and before | None | No | Yes |
| v0.33.45 (PR #299) | `beforeunload` fetch | Yes | No (brief ungrouped flash) |
| v0.33.46 | `on_before_close` Rust hook | No (lookup failure) | Yes |
| Target fix | `on_before_close` + `browser.identifier()` key | Yes | Yes |
