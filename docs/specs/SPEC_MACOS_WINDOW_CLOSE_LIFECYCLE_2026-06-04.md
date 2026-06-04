# SPEC: macOS window-close lifecycle — "closes but stays open hidden"

**Status:** Draft · **Date:** 2026-06-04 · **Owner:** AgentMux host/macOS
**Related:** `agentmux-cef/src/main.rs` (shutdown), `agentmux-cef/src/client/mod.rs`
(`do_close`/`on_before_close`/last-window quit), `agentmux-cef/src/ui_tasks.rs`
(`CloseWindowTask`), `agentmux-launcher/src/splash_mac.rs` (supervisor lifetime),
`SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`

---

## 0. Symptom

On macOS, clicking the window **close (red traffic-light) button** makes the window
**disappear**, but the app does **not actually quit** — it "appears open but hidden":
the process keeps running with no visible window. Expected: the window closes and, when
it is the last window, the instance fully exits (its Dock tile and process go away).

This is a **lifecycle / teardown** bug, not a rendering one: something orders the window
out (hides it) while the process fails to reach a clean exit.

---

## 0.1 Resolution (confirmed 2026-06-04)

Diagnosed live with a `[shutdown-diag]` trace around each shutdown step. The close → quit
path is entirely correct; the wedge is a refined **H1**: the host hangs in the teardown,
specifically at **`drop(runtime)`** (`main.rs`). The trace reached `cef shutdown() returned`
(CEF shutdown is fine, ~50 ms) and then **never** reached `tokio runtime dropped` — the
multi-thread tokio runtime's drop blocks forever waiting for every background task /
blocking thread to finish, and at least one `spawn_blocking` (a pipe/PTY reader) is parked
on a blocking read that never returns. The window is already destroyed, so it "closes," but
the host wedges in the runtime drop and never exits; the launcher (which exits only when it
observes the host exit) wedges with it → "open but hidden."

The earlier reverted `process::exit(0)` was placed *after* the (never-reached) "shutdown
complete" log, i.e. *past* the wedge — which is why it never helped.

**Fix (shipped):** on macOS, replace the blocking `drop(runtime)` with
`runtime.shutdown_background()` (non-blocking) followed by `std::process::exit(0)` after the
graceful CEF shutdown + sidecar kill + port-file cleanup. Reaching this code only happens on
the intended `LastWindowClosed` quit (the CEF message loop returned), so a hard exit is
correct, not a crash. Verified: the host reaches `AgentMux host shutdown complete (fast
exit)` and the whole instance (host + launcher + srv) exits cleanly in ~60 ms, no lingering
processes. Other platforms keep the plain `drop(runtime)`.

---

## 1. The close path today (what *should* happen)

1. **Frontend:** the macOS traffic-light close calls `getApi().closeWindow()`
   (`frontend/app/window/window-controls.darwin.tsx`).
2. **IPC → host:** `close_window` (`commands/window/lifecycle.rs:38`) → on non-Windows,
   `ui_tasks::post_close_window(label)`.
3. **UI task:** `CloseWindowTask` calls `host.try_close_browser()` (deliberately not
   `window.close()` — see the comment at `ui_tasks.rs:64`, which avoids a
   `Widget::Close` CHECK abort when macOS already sent `windowShouldClose`).
4. **CEF callbacks** (`client/mod.rs`):
   - `do_close` returns `false` → **allows** the browser to be destroyed (no hide).
   - `on_before_close` runs the pane-reaper teardown. When the **user-visible** browser
     count drops to 0 (`user_browser_count == 0 && !is_browser_pane`), it sets
     `quit_state = Draining` (reason `LastWindowClosed`) and, once the browser list is
     empty, calls `quit_message_loop()`.
5. **Shutdown** (`main.rs:842-874`): `run_message_loop()` returns →
   `wrr::uninstall_hooks()` → kill backend sidecar → `shutdown()` (CEF) →
   `drop(runtime)` → remove port-file → log `AgentMux host shutdown complete` →
   **`main` returns** (process is expected to exit here).
6. **Launcher** (`splash_mac.rs`): the supervisor thread "owns process lifetime and
   `process::exit`s when the host exits." So the launcher exits *because* it observes the
   host exit.

So the intent is correct: close → (if last) quit → process exits → launcher exits.

---

## 2. Root-cause hypotheses (ranked)

### H1 — The process never fully exits after `main` returns (most likely)
On macOS a process exits when `main` returns **only if** no non-daemon thread and no live
run loop keeps it alive. CEF/Chromium + the launcher IPC reader + WRR + any un-joined
thread can keep the process resident. `main.rs:873-874` logs `shutdown complete` and then
just **returns — there is no `std::process::exit(0)`**. A macOS force-exit at exactly this
point existed earlier and was **reverted** (during the splash/tear-off work). Without it,
the host can linger after teardown.

Consequence chain: window is `orderOut`'d by AppKit's close, `quit_message_loop` returns,
teardown runs, but the **process stays resident** → "open but hidden." And because the
launcher supervisor only exits *when it sees the host exit*, the launcher lingers too —
so the whole instance stays up, windowless.

**Why the earlier force-exit was reverted matters:** it likely looked like a non-graceful
exit to the launcher's saga/health logic, or it raced the splash-removal runloop. Any
fix must (a) run *after* the graceful teardown in §1.5, and (b) signal the launcher this
is an intended quit, not a crash (so it doesn't respawn).

### H2 — `orderOut` without destroy (hide masquerading as close)
AppKit's red-button path (`-[NSWindow performClose:]`/`windowShouldClose:`) may order the
window out (hide, with animation) on a path that races our CEF `try_close_browser`. If the
CEF destroy is deferred or dropped (e.g. the task is queued but the message loop is already
draining — see the `last-window-closed` task-drop note at `ui_tasks.rs:670`), the NSWindow
ends up hidden while the browser is never destroyed → "open but hidden," and the
last-window quit never triggers because the browser count never reaches 0.

### H3 — A second top-level keeps `user_browser_count > 0`
A **floating/torn-off pane window** or a **pre-warmed pool window** that is counted (or a
hidden off-screen window) keeps `user_browser_count > 0`, so closing the visible window
never satisfies the last-window condition. The instance stays alive with only a
hidden/off-screen window present. (Pool windows are supposed to be excluded; verify they
actually are on this path.)

### H4 — Launcher respawns the host
If the host *does* exit but the launcher's supervisor/saga interprets the exit as a crash
(non-zero, or unexpected), it respawns the host, which then has no window to show →
appears "open but hidden." (Less likely given the splash_mac comment, but must be ruled
out for the `LastWindowClosed` quit reason specifically.)

### H5 — macOS "last window ≠ quit" convention, half-applied
Standard macOS keeps the app alive after the last window closes. AgentMux deliberately
quits instead (§1.4). If only *part* of that intent fires (window closed, quit path not
reached), you get the windowless-but-alive state. This overlaps H1/H3.

---

## 3. Investigation plan (confirm before fixing)

1. **Reproduce + observe the process.** Click the red close; immediately `ps`/Activity
   Monitor for `agentmux-cef` and `agentmux-launcher`. Determine: does the host process
   stay? the launcher? Capture the host log tail — do we see `quit_message_loop returned`
   and `AgentMux host shutdown complete`?
   - If we **do** see `shutdown complete` but the process stays → **H1** (clean teardown,
     no final exit).
   - If we **don't** reach `quit_message_loop` → **H2/H3** (browser never destroyed / count
     never hits 0). Add a one-line log of `user_browser_count` + `browsers_keys` on the
     close.
   - If a **new** host PID appears → **H4** (respawn).
2. **Enumerate live browsers/windows at close** (`browsers_keys`, `pool_keys`,
   `is_browser_pane`) to test H3 — confirm pool/floaters are excluded from the count.
3. **Thread audit** for H1: list live threads after `shutdown complete` (which thread keeps
   the process up — launcher_ipc reader? a tokio worker? an NSApp observer?).
4. Repeat with **multiple windows / a torn-off pane open** to cover H3 explicitly.

---

## 4. Proposed fix (pending §3 confirmation)

Layered, smallest-correct-first:

- **If H1 (process lingers after a clean teardown):** after the §1.5 graceful sequence,
  on macOS explicitly terminate the process — but do it *right*:
  - run it strictly **after** `shutdown()` + sidecar kill + port-file cleanup, gated on the
    `LastWindowClosed`/intended-quit `QuitReason` (never on a crash path);
  - tell the launcher this is an **intended** exit first (a `Goodbye`/clean-exit signal on
    the launcher pipe) so the supervisor exits with us and does **not** respawn;
  - then `std::process::exit(0)`. This restores the reverted behavior but scoped and
    sequenced so it can't mimic a crash or race the splash runloop.
  - *Preferred alternative if feasible:* find and join/cancel the lingering thread so a
    bare `main`-return exits cleanly (no force-exit). Costlier; do only if the lingering
    owner is a small, well-understood thread.
- **If H2 (hide-without-destroy / dropped close task):** ensure the close actually destroys
  the browser even when the message loop is draining — don't drop the `CloseWindowTask`
  during last-window-closed; reconcile the AppKit `performClose:` path with
  `try_close_browser` so exactly one destroy happens and the NSWindow isn't left
  orderOut-but-alive.
- **If H3 (count never hits 0):** correct the last-window predicate so pool/floating/hidden
  windows are excluded (or closed) and the real last user window triggers the quit.
- **If H4 (respawn):** make the launcher treat the `LastWindowClosed` host exit as terminal
  (no pool-respawn / health-restart for that quit reason).

**Acceptance:** clicking the close button on the last window → window gone **and** both
`agentmux-cef` and `agentmux-launcher` processes exit (verified via `ps`), Dock tile gone,
no respawn, within a defined budget; with multiple windows, closing a non-last window
leaves the others intact and the process alive.

---

## 5. Cross-platform / regression notes

- Windows uses `PostMessageW(hwnd, WM_CLOSE)` and its own last-window path; keep this
  macOS-scoped (`#[cfg(target_os = "macos")]`) so Windows/Linux are unaffected.
- Don't reintroduce the `Widget::Close` CHECK-abort that `CloseWindowTask` already avoids
  (`ui_tasks.rs:64`), or the macOS tear-off drag crash work
  (`SPEC_MACOS_TEAROFF_STABILITY`).
- Multi-instance: a force-exit must terminate **only this instance's** host+launcher, never
  another running AgentMux (respect the isolation invariants — the launcher owns only its
  own job/process).

---

## 6. Open questions

- Exact reason the prior macOS `std::process::exit(0)` was reverted (git history /
  `docs/retro/b9-3-quit-thread-analysis.md`) — what broke, so we don't repeat it.
- Is the symptom intermittent (race) or deterministic? (Earlier in dev, last-window-close
  *did* exit cleanly — so this may be state-dependent: multi-window, a torn-off pane, or a
  pool window present.)
- Should AgentMux instead adopt the macOS convention (stay alive, no quit on last window)
  with an explicit Quit — or keep "close last window = quit"? (Product decision; this spec
  assumes the current "quit on last window" intent and fixes it to actually exit.)
- Which thread/run loop keeps the process resident after `main` returns (the H1 owner)?

---

## 7. References

- `agentmux-cef/src/main.rs:840-874` (message loop + shutdown sequence; no final exit)
- `agentmux-cef/src/client/mod.rs` `do_close` (652), `on_before_close` (712), last-window
  quit (943-1126, `quit_state = Draining`, `quit_message_loop`)
- `agentmux-cef/src/ui_tasks.rs:56-82` (`CloseWindowTask` / `try_close_browser`), `:670`
  (task-drop during last-window-closed)
- `agentmux-cef/src/commands/window/lifecycle.rs:38` (`close_window`)
- `agentmux-launcher/src/splash_mac.rs:232` (supervisor owns process lifetime)
- `docs/retro/b9-3-quit-thread-analysis.md` (prior quit-thread analysis)
