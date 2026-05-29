# Report — macOS tab/pane tear-off crashes the host with `EXC_BREAKPOINT` / unrecognized selector

**Date:** 2026-05-29
**Repo state:** `main` @ `53d781af` (v0.40.0) + PR-in-flight macOS fixes (`agenta/macos-cef-framework-bundling`, HEAD `d4bd036f`)
**Author:** AgentO-asaf
**Status:** Diagnostic report, no patch — macOS tear-off is intentionally unimplemented and needs Phase 7 / Phase C1 work to make any drag-initiating gesture safe.
**Related issue:** [#1138](https://github.com/agentmuxai/agentmux/issues/1138) (initial crash filing — same root cause; this expands it with code-path evidence)
**Related specs:**
- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` — explicit "Out of scope: Floater on macOS / Linux. Windows-only initially."
- `docs/specs/PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md` — "Scope: Win32 only. macOS / Linux defer to Phase 2."
- `docs/specs/RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md` — confirms there is no portable tear-off path; each OS needs a bespoke drag loop.

---

## TL;DR

Every tab/pane tear-off attempt on macOS crashes `agentmux-cef` (the host) with an uncaught Objective-C `unrecognized selector` exception, surfacing as `EXC_BREAKPOINT` / `SIGTRAP` on the main thread. The Rust side is innocent — the macOS branches in `commands/drag.rs`, `commands/tear_off_hook.rs`, and `commands/window_pool.rs` are intentionally no-op stubs marked for Phase 7. The crash originates in **CEF's macOS code** when Chromium tries to drive a drag/drop session through an `NSWindow`/`NSView` subclass that AgentMux never set up. The cross-platform spec confirms macOS is out of scope for current floating-pane work; this report documents the gap so a future Phase 7 / C1 implementation has a starting point.

There are six distinct crash diagnostics on this machine from `task dev` sessions yesterday and today, all with identical signatures. The crash is 100% reproducible by attempting to drag any pane header.

---

## Crash signature (identical across all six diagnostic reports)

From `~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-29-094108.ips` (and five other reports from 2026-05-28):

```
Exception:       EXC_BREAKPOINT (SIGTRAP) — (Breakpoint) brk 1
Faulting thread: CrBrowserMain (com.apple.main-thread)
asi[0]:          "unrecognized selector sent to instance %p"
asi[1]:          "unrecognized selector sent to instance 0x118001a3f00"
```

Top of the stack (Cocoa exception-handling path):

```
+[NSApplication _crashOnException:]              + 256
-[NSApplication reportException:]                + 460
NSApplicationUncaughtExceptionHandler            + 152
__handleUncaughtException                        + 820
_objc_terminate()                                + 144
std::__terminate(void (*)())
__exceptionPreprocess                            + 164
objc_exception_throw                             + 88
+[NSObject(NSObject) instanceMethodSignatureForSelector:]
___forwarding___                                 + 1480
_CF_forwarding_prep_0                            + 96
ChromeWebAppShortcutCopierMain  <CEF code>
... <CEF event loop>
[NSApplication run]
```

The crash report's `asi[]` only captures `"unrecognized selector sent to instance %p"` (the format string) — not the actual selector name. **Without an `NSExceptionHandler` installed before the throw, we can't see which method CEF called that AgentMux's window doesn't respond to.** Capturing this is the first action item in §7 below.

The Rust process never gets a chance to log a panic. AppKit's `_crashOnException` calls `_objc_terminate()`, which terminates the process without unwinding — the host log stops mid-stream (last entries are usually a `[fe] focusedChild focus`, `[ipc] main_window_focus`, or `WaveObj updated layout` line, depending on what the user was doing when they initiated the drag).

---

## What the host code actually does on macOS

### 1. Tear-off RPC is gated to Windows; macOS gets a no-op stub

`agentmux-cef/src/commands/tear_off_hook.rs`:

```rust
#[cfg(not(target_os = "windows"))]
pub fn start_tear_off_tracking(
    _state: std::sync::Arc<crate::state::AppState>,
    _source_label: String,
    _dragged_label: String,
    _tab_id: String,
    _source_ws_id: String,
    _dest_ws_id: String,
    _original_tab_index: usize,
    _was_pinned: bool,
    ...
) { /* no-op */ }
```

`agentmux-cef/src/commands/drag.rs`:

```rust
#[cfg(not(target_os = "windows"))]
fn hit_test_windows(_state: &Arc<AppState>, _screen_x: f64, _screen_y: f64) -> Option<String> {
    None
}

// ... handshake_ms computation:
#[cfg(not(target_os = "windows"))]
let handshake_ms: f64 = {
    // Phase 7 adds macOS (NSWindow performWindowDragWithEvent) +
    // Linux (_NET_WM_MOVERESIZE / xdg_toplevel.move) equivalents.
    // For now the non-Windows path is a no-op so the IPC contract
    // exists and the rest of the pipeline can be cross-platform.
    let _ = (state, &dest_label);
    0.0
};
```

So the Rust side neither initiates a drag loop nor talks to AppKit. The IPC contract is preserved, the call returns immediately, the renderer carries on.

### 2. Frontend uses HTML5 drag + pointer capture; CEF handles the rest

`frontend/app/tab/droppable-tab.tsx` sets up the drag handlers; the comments mention the "cross-window tear-off path" but the actual native window-positioning is delegated to CEF / the host. On Windows, `tear_off_hook` takes over with `SetWindowPos` per `RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md` §3.2 (path-2, the Win32 custom drag loop). On macOS, with the host stub doing nothing, the drag operation falls through to **Chromium's own macOS code path** — which is where the unrecognized-selector throw happens.

### 3. CEF's macOS implementation expects an `NSWindow` subclass with drag/pasteboard delegate methods

CEF for macOS forwards drag events through an `NSDraggingSource` / `NSDraggingDestination` machinery rooted at the application's window. When CEF dispatches one of those delegate selectors (e.g. `draggingEntered:`, `draggingUpdated:`, `prepareForDragOperation:`, `performDragOperation:`, `draggingEnded:`), the receiver in the AgentMux process is the `NSWindow` instance CEF created via `cef::Window` — and that window doesn't subclass anything that implements the expected drag protocol. The selector dispatch goes through `___forwarding___`, fails to find a handler, and `objc_exception_throw` fires.

`grep -rn "NSDraggingDestination\|NSDraggingSource\|registerForDraggedTypes\|prepareForDragOperation\|draggingEntered" agentmux-cef/src/` returns **zero** matches. AgentMux has no native AppKit code at all — it relies entirely on whatever CEF's cef-rs wrapper provides. cef-rs 146.7.0's `cef::Window` doesn't expose a way to install a custom NSDraggingDestination, and CEF's internal NSWindow subclass apparently assumes the embedder will install one.

### 4. The `floating_pane` module exists but only implements the Windows primitive

`agentmux-cef/src/floating_pane.rs` is the host primitive for the floating-pane tear-off (separate from tab tear-off). It builds the floating window via Win32 APIs (`SetWindowLong`, `WM_NCHITTEST`, etc.). The `SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` spec explicitly lays out the macOS equivalent (`NSPanel` + `addChildWindow:ordered:` + child-window focus chain → see spec §3.2) and tracks it as **C1**, ~400 lines, "macOS-only; mirrors Windows Phase 1 structure". `floating_pane_macos.rs` (or `mod macos` inside the existing module) is unimplemented; the issue is referenced in the spec's open work list.

---

## Why this crash is not caused by the in-flight macOS PR work

Quick reality check on the surrounding PR-in-flight work (`agenta/macos-cef-framework-bundling`, the branch this report sits on):

| Change | Could it cause this crash? |
|---|---|
| `bundle:darwin` Framework copy + GL libs | No — affects startup library loading only |
| `framework_dir_path` / `resources_dir_path` in CefSettings | No — affects pak/locale resolution at init, not drag |
| `password-store=basic` + `use-mock-keychain` | No — OSCrypt subsystem only |
| `patched-libcef` feature gate around `begin_window_drag` | No — that call site is `#[cfg(not(target_os = "windows"))]` AND requires a feature flag we don't enable |
| `on_pre_key_event` signature per-platform split | No — keyboard handler only, never invoked from drag |
| Traffic-light `WindowControlsLeft` + `--default-indent: 0px` then back to 10px | No — pure DOM/CSS; the receiver in the crash is a `NSWindow`, not any HTML element |
| `-webkit-app-region: drag` on `.window-header` (today's drag-fix commit) | **Possibly**, see below. |

The drag-fix CSS change (`window-header.darwin.scss` adding `-webkit-app-region: drag`) is the one place where my work intersects with macOS native drag. After that commit, Chromium's `on_draggable_regions_changed` callback fires (verified in `agentmux-host-v0.40.0.log.2026-05-29` at `16:35:26.379731Z`: "13 regions — \"1200x33@0,0 drag=true\"…"), and the Rust side calls `cef::Window::set_draggable_regions(regions)`. That call may, transitively, exercise the same AppKit code path that ends up looking for a drag-protocol selector on the AgentMux NSWindow.

However:

- The crash signature predates that commit. Yesterday's six crash reports from 2026-05-28 are pre-traffic-lights and pre-drag-CSS, and have the identical signature. So **the underlying NSException bug is not caused by my CSS**; it's structural.
- My CSS may have *widened* the window of opportunity (now any title-bar drag attempts the system path, where previously only pane-header tear-off did). That's a tractable workaround: revert the drag CSS so the host doesn't tempt CEF into the broken path on every window-title-bar drag. Users would lose window drag again (which they didn't have anyway pre-commit), but pane drag would still crash on its own initiative.

---

## Reproduction

100% reproducible on this branch with the v0.40.0 host:

1. `rm -f ~/.agentmux/dev/<branch>/<hash>/cef-cache/{SingletonLock,SingletonCookie,SingletonSocket,ipc-port}` to clear singleton turds from prior crashes.
2. `task dev` — wait for "✅ Main application loaded successfully".
3. Click and drag any pane header more than a few pixels.
4. Host exits in ≤200 ms. `agentmux-srv` survives, orphaned. The host log ends mid-event; a new `.ips` lands under `~/Library/Logs/DiagnosticReports/agentmux-cef-<timestamp>.ips`.

Also reproducible by attempting tab tear-off (drag a tab out of the tab bar), and reproducible just-by-loading on the v0.40.0 build at `16:41:08Z` today — that one is a single-click somewhere followed by Chromium's own delayed drag setup, suggesting some autorelease-pool / runloop-source path also triggers the same selector miss.

---

## Side effects on `task dev` UX

Each crash leaves a stale `SingletonLock` symlink in the CEF cache pointing at the dead PID. The *next* `task dev` invocation does a Chromium process_singleton handoff to that dead PID and exits with code 24 — making the app look like it "won't start". The fix is the manual `rm` above, but **the experience is awful**: a single accidental drag costs a full host restart cycle including a confusing "AgentMux failed to start" banner if the user clicks Reload.

Recommendation: add a small launch-side guard that prunes the lock if the named PID is gone (`kill -0 <pid>`), so a stale singleton from a prior crash gets cleaned up by `task dev` itself. Five lines in `dev:serve`'s shell snippet.

---

## What's needed to fix this properly

Two layers, in order:

### Layer 1 — Stop the bleeding so dev is usable on macOS today

**Option A: defensive `NSWindow` subclass / category.** Install an early `responds_to_selector:`-defaulting override on the AgentMux NSWindow so missing drag-protocol selectors return `nil` instead of throwing. At worst this makes drag a no-op instead of fatal. Probably 30–50 lines of `objc2` from Rust, gated `#[cfg(target_os = "macos")]`, hooked into `main.rs` right after CEF init.

Tradeoff: this masks the real bug. We don't know which selector is missing; we'd hide the crash but tear-off and any future feature dependent on Cocoa drag delegate methods would still silently misbehave.

**Option B: install an `NSExceptionHandler` early.** Trap the throw, log the selector name + class + stack, then either re-raise (preserving the crash for diagnostics) or swallow (making it a soft error). This is the **prerequisite** for any real fix because we currently don't know which selector AgentMux's NSWindow is missing.

**Option C: revert `-webkit-app-region: drag` on macOS.** Returns drag to "not implemented" (as it was on `main` before this PR). User-visible loss: no window drag from the title bar. Pane/tab tear-off still crashes the same way it did before this PR, so this doesn't fix the original report — it just narrows the surface area to "deliberately initiated tear-off gestures crash" rather than "system-driven drag also triggers it on autorelease."

Recommendation: **(B) first** — install the exception handler so we have data — then revisit whether (A) is a sufficient guard once we know the selector. (C) is independent and only relevant if (B) reveals that the autorelease/draggable-regions path is a *separate* selector miss from the tear-off path.

### Layer 2 — Build real macOS tear-off (long-running)

This is the **C1** work block from `SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`:

- Implement `agentmux-cef/src/floating_pane_macos.rs` (or `mod macos` inside the existing module) with `NSPanel` + `addChildWindow:ordered:`.
- Wire `commands::drag::start_tear_off_tracking`'s `#[cfg(target_os = "macos")]` branch to call into the new module and drive `NSWindow.performWindowDragWithEvent:` instead of the no-op stub.
- Implement an `NSDraggingDestination`-conforming view AgentMux owns, so the receiver of the drag-protocol selectors is *our* view, not whatever default object CEF leaves in place. This is also what prevents the unrecognized-selector throw.

Per the spec, this is "~400 lines, Medium effort, mirrors Windows Phase 1 structure." Not in scope for the current PR.

---

## Action items

1. **Install `NSExceptionHandler` in `agentmux-cef/src/main.rs`** (early, before `cef_initialize`). Log selector name + class + receiver address + stack to the tracing layer. Re-raise. — *Smallest amount of work; biggest diagnostic payoff.*
2. **Update [#1138](https://github.com/agentmuxai/agentmux/issues/1138) with this report** and link to `SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md` so the work block is connected to the user-visible crash.
3. **Add `dev:serve` SingletonLock pruning** so a stale `.../SingletonLock -> hostname-<dead-pid>` symlink gets cleaned up automatically on the next launch. Five lines in `Taskfile.yml`. Massively improves the UX of recovering from any host crash on macOS.
4. **(Optional)** Decide whether to revert the `.window-header` `-webkit-app-region: drag` on macOS until (1) gives us enough info to know if it's a different selector. The drag CSS shipped today restores title-bar window drag (the feature the user noticed was missing) but increases the surface area of CEF's drag code path; without (1) we can't tell if that surface area also triggers the same throw.

---

## Appendix: the six crash diagnostics on this machine

```
~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-29-094108.ips
~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-28-120731.ips
~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-28-111209.ips
~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-28-110037.ips
~/Library/Logs/DiagnosticReports/agentmux-cef-2026-05-28-...                 (and others)
```

All identical signatures. Today's (post-traffic-lights + drag CSS) is structurally identical to yesterday's (pre-traffic-lights + no drag CSS). That's the strongest evidence that the bug pre-exists the in-flight macOS PR work and is owned by the missing native tear-off implementation.
