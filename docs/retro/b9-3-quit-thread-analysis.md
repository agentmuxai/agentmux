# B.9.3 — analysis of the "host won't quit" loop

**Status:** Analysis, written 2026-04-29 after v0.33.491–v0.33.493 smoke iterations all left the host process tree alive.
**Author:** AgentA.
**Goal:** stop oscillating; establish the real constraints, the real failure mode, and the right fix.

---

## What's actually happening

### Smoke results (each test: open AgentMux → tear off → close all visible windows)

| Build | Approach | Observed |
|---|---|---|
| v0.33.491 | tokio handler posts a UI task that reaps pool + calls `quit_message_loop` | Task body **never executed**. `[wrr] HostShouldQuit received` logs from tokio handler; nothing from inside the task. Host stays alive. |
| v0.33.492 | tokio handler calls `cef::quit_message_loop()` directly | Call **returns immediately, no effect**. Host stays alive. |
| v0.33.493 | tokio handler posts a minimal UI task that does ONLY `quit_message_loop` | Same as 491. Task body never logs. Host stays alive. |

### Two independent things are broken — naming them precisely

1. **Cause A: the host's own close path (`client.rs:515`) doesn't fire `quit_message_loop` when it should.** This existed before B.9. The whole reason B.9.3 was added is that this gate fails to trigger reliably. Specifically: when the user closes the last visible window, `on_before_close` runs and counts user-facing browsers (filtering pool + browser-pane). The gate is `user_browser_count == 0 && !self.is_pane`. We have evidence that this path never logs `last user-facing window closed` despite the user-visible state being "all closed". Possible reasons (ranked by likelihood, unverified):
   - The closing browser's client has `self.is_pane == true` (the gate skips non-main-clients deliberately, per "only the main client should trigger app exit"). If the close happens to land on a pane-client, the quit is silently skipped.
   - Pool refill races the count check: a new pool window has been added to `state.browsers` but not yet to `unpromoted_pool_labels`, so it counts as user-facing.
   - The user-facing count is correctly 0 but `quit_message_loop` is somehow itself a no-op in this state (less likely; this is the function CEF supplies for exactly this purpose).

2. **Cause B: `cef::post_task(ThreadId::UI, ...)` silently drops tasks during the post-close-cascade window.** This is what made the B.9.3 fix attempts fail. We confirmed empirically that:
   - Earlier in the same session, tasks posted via `post_task` DO run (e.g. `MainFocusReclaimTask` ran 1–4 seconds before HostShouldQuit fired).
   - After the close cascade, our newly-posted task — even a one-line task — never executes, never logs.
   - The post call itself doesn't return an error or panic. The task is enqueued but the queue is not drained.

These are SEPARATE bugs. B.9.3's fix was supposed to work around Cause A, but my fix attempts ran into Cause B.

---

## What CEF actually requires (the "thread" question)

I oscillated between "must be on UI thread" and "thread-safe, call from anywhere" over three iterations. Here is the actual answer.

CEF C API doc for `cef_quit_message_loop`:

> Quit the CEF message loop that was started by calling `cef_run_message_loop()`. This function should only be called on the main application thread and only if `cef_run_message_loop()` was used.

This is unambiguous: **UI thread only**. The function called from any other thread is undefined behavior — in practice, on Windows, it's a silent no-op (the v0.33.492 evidence).

So calling from the tokio thread (v0.33.492) was wrong. I knew it was wrong; I tried it anyway hoping the binding might shim. It doesn't.

The right pattern is: **the work runs on the UI thread; the trigger is delivered from any thread via a thread-safe bridge.**

CEF provides one such bridge: `cef::post_task(ThreadId::UI, task)`. It IS thread-safe to call. But it requires CEF's task queue to be functional, and our smoke evidence shows it isn't functional in the specific window between "last visible browser closed" and "host process exits". That's Cause B.

Win32 provides a different bridge: `PostThreadMessage(thread_id, msg, ...)`. Documented thread-safe. Goes through Windows' message queue (NOT CEF's task queue). `WM_QUIT` posted via this terminates `GetMessage` → `run_message_loop` returns → host falls out of `main()` → process tree exits via J0.

So the choice isn't "UI thread vs not UI thread". Both bridges DELIVER the work to the UI thread. The choice is "CEF's task queue (proven unreliable here) vs Windows' message queue (proven reliable)".

**`PostThreadMessage(WM_QUIT)` is the correct bridge** for this specific state because:
- Win32's message queue is at a layer below CEF's task queue
- `cef_run_message_loop`'s implementation on Windows is a `GetMessage` loop; `WM_QUIT` is the canonical termination signal
- Even if CEF stops draining its own task queue during teardown, `GetMessage` continues to run until it sees `WM_QUIT`

---

## Recommended path forward

### B.9.3.1 (current iteration, building as v0.33.494)

Land the Win32 `PostThreadMessage(WM_QUIT)` approach. It's:
- The right fix for Cause B (CEF task queue unreliable in this window)
- A defensible permanent design (Win32 message-pump is more fundamental than CEF's task queue; we're not papering over a CEF bug, we're using a lower-level primitive that's appropriate for "tell the message loop to exit")
- Independent of Cause A (the host's broken gate)

Specifically:
- `wrr/win_event.rs::install_hooks` captures `GetCurrentThreadId()` (this runs on the host's main thread = UI thread)
- `wrr/win_event.rs::post_thread_quit_message()` reads the captured TID and calls `PostThreadMessageW(tid, WM_QUIT, 0, 0)` from any thread
- `launcher_ipc::apply_event_to_shadow` for `Event::HostShouldQuit` calls this (post race-window check)

### B.9.3.2 (separate, defensive, follow-up)

Investigate Cause A — why `client.rs:515 quit_message_loop()` doesn't fire on the existing close path.

Hypotheses to test:
1. Add a log line right before the `user_browser_count == 0 && !self.is_pane` check showing `user_browser_count`, `is_pane`, the browsers map keys, and the filtered set. Re-smoke and read the log.
2. If `is_pane == true` is the culprit: the gate's "only main client triggers exit" assumption is wrong; the LAST-CLOSE check should run on whichever client owns the last user-visible browser, regardless of `is_pane`.
3. If `user_browser_count != 0` due to pool refill: the count needs to use a different snapshot of `unpromoted_pool_labels` — perhaps "pool labels we've EVER seen, minus those promoted" rather than the current set after refill.

This is host-internal cleanup and should happen anyway. B.9.3.1 already provides safety from the launcher-side, so the host fix can be unhurried.

---

## Decision rules going forward (so the oscillation stops)

1. **The destination is the UI thread.** Always. CEF docs are right.
2. **The bridge is the choice.** `cef::post_task(TID_UI, ...)` is the default; switch to Win32 `PostThreadMessage`/`PostMessage` when (a) there's evidence the CEF task queue isn't draining, or (b) we need the work to happen below CEF's abstraction (e.g., to terminate the message loop itself).
3. **Don't call CEF UI-thread-only APIs from a worker thread directly.** It's not "thread-safe in our binding"; it's silent UB. The v0.33.492 attempt was wrong on first principles and shouldn't have been tried.
4. **When a UI task doesn't execute, log a stable signal in the task body.** Then we can tell "task ran but I'm misreading the log" from "task never ran" — which is what cost us iteration 491→493. (Done in v0.33.493 by stripping the task to one log line; the silence was the answer.)

---

## What we actually learned

- The `OrphanInstance` reducer arm + `HostShouldQuit` saga are correct. The reducer detects the transition cleanly. The host receives the event cleanly. The IPC + reducer + saga emission all work end-to-end.
- The failure is purely in "how does the launcher-side saga deliver work to the host's UI thread when CEF's task queue is unreliable in this window".
- Win32 `PostThreadMessage` is the answer; not because it bypasses the UI thread (it doesn't — it lands on it via the OS message queue), but because it bypasses CEF's task queue (which is the thing actually broken here).
