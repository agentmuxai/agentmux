# macOS Tab Redock Parity — Implementation Scoping

**Date:** 2026-07-24 (revised same day — see §0.1 "Scope correction")
**Status:** implemented — PR #2310 (CGEventTap tracking); §5 landing-bounce animation remains a known follow-up. Verified 2026-08-10.
**Trigger:** User: *"lets work on the tab redock .. i believe it
  was implemented for windows by not macos"* → confirmed via
  investigation → *"which has better performance? engineering cost
  is not important"* → user chose full native parity over the
  cheap frontend-only interim fix.
**Constraint carried over from the parent spec (§0):** time/expense
  is not a factor; no MVP cuts; macOS must reach *perceptual* parity
  with Windows, not "good enough."
**Supersedes:** the `[NSWindow performWindowDragWithEvent:]`
  prescription in the parent spec's §6/§7 (moot now — see §0.1, but
  the reasoning in §1 is kept as it may matter again if the
  live-follow tear-off model is ever revived).

---

## 0.1 Scope correction (read first)

The first draft of this doc scoped two phases: (7a) porting the
Win32 `SC_MOVE` live-cursor-follow tear-off window-move to macOS,
and (7b) the `CGEventTap` cross-window hit-test/merge-detection
hook. **7a turned out to be unnecessary — it would have ported dead
code.**

Tracing the actual call graph: `requestTearOff`'s `skipScMove`
parameter has exactly one call site, `createTearOffTabAtRelease`
(`frontend/app/tab/tab-tearoff-rpc.ts`), which always passes
`skipScMove: true`. The `else` branch that calls
`tearOffSCMoveHandshake` — and everything downstream of it
(`start_tear_off_tracking`, `HookMode::TearOff`, the
`tearoff:hover-*`/`tearoff:merge`/`tearoff:cancel-back`/
`tearoff:standalone` events) — is unreachable **on every platform,
including Windows.** Per the commit history, Windows itself moved to
a "commit-on-release" tear-off model in the most recent tab-drag
work (`fix(tabs): commit-on-release tear-off + loosen reorder + kill
drag circle-slash (Windows) (#2175)`, 2026-07-16): the torn-off
window is created directly at the release point, full stop — no
live cursor-follow anymore. macOS's existing `dragend`-based
tear-off (`CrossWindowDragMonitor.darwin.tsx`) already does the same
thing (window appears at the drop point). **Tear-off window
placement is already at parity between the two platforms.** There is
no gap there to close, and no live-follow window-move work is
needed on macOS.

**What's actually still live on Windows and missing on macOS:**
`HookMode::TabDrag` only — the in-strip drag → direct cross-window
merge, with a live cursor-accurate insertion indicator, confirmed
still active today (`tabdrag:merge-direct` has real dedup logic
against the legacy HTML5 fallback in `tabbar-dnd.ts`'s
`markTabMerged`/`wasTabRecentlyMerged`). This is the literal "tab
redock" gesture from the user's original ask, and — importantly —
**it doesn't need any window-move loop at all.** The tab strip's own
HTML5 drag session already handles the in-window reorder visuals;
the hook's only job is watching where the cursor is *relative to
other AgentMux windows* while that ordinary drag is in progress, so
a drop outside the source window can be redirected into a direct
cross-window merge instead of falling through to the append-only
`DragOverlay` fallback.

**Revised scope: this doc is now just the `CGEventTap` hit-test/
merge-detection hook for `HookMode::TabDrag`, plus the Accessibility
permission subsystem it depends on, plus validation.** Sections 1
and 2.5 (the `run_macos_native_drag_loop` prior-art writeup) are
kept because the reasoning is still correct and may be relevant if
this codebase ever revives a live-follow tear-off model — but no
code from that section ships as part of this plan.

---

## 0. What "tab redock" means here (scope boundary)

**In scope:** in-strip tab drag → drop on another window's strip.
No tear-off window is ever created; the tab, while still being
dragged inside its source strip's HTML5 drag session, is inserted
directly into the destination window's strip at the cursor-indicated
position on drop. Windows mechanism: `HookMode::TabDrag` in
`tear_off_hook.rs`.

**Not in scope:**
- Tear-off-then-drag-to-merge (`HookMode::TearOff`) — dead on all
  platforms per §0.1, nothing to port.
- Linux/X11 or Wayland — tracked separately by the parent spec
  (user: *"ill have the linux agent do linux"*); this doc is
  macOS-only.
- Cross-process drag (dragging a tab into Chrome/Slack) — parent
  spec NG1, unchanged.

---

## 1. Prior art already in this codebase (context, not shipped code)

Kept for reference in case a future live-follow tear-off model is
revived (see §0.1) — **not part of this plan's deliverable.**

`ui_tasks/drag.rs::run_macos_native_drag_loop` (whole-window drag for
the floating-pane redock header) explicitly rejects
`performWindowDragWithEvent:` with a documented reason: that API
needs the *original* mouse-down `NSEvent`, but its trigger arrives
async over IPC, so `[NSApp currentEvent]` doesn't hold the original
event and the AppKit drag can silently fail to start. The proven fix
there: pump `NSLeftMouseDragged`/`NSLeftMouseUp` manually via
`[NSApp nextEventMatchingMask:...untilDate:...inMode:
NSEventTrackingRunLoopMode dequeue:YES]` and reposition via CEF's own
`Window::set_bounds`. This pattern is unrelated to the `CGEventTap`
work below (that's cross-window cursor *tracking*, not a window
*move* loop) — it's noted here only so a future implementer doesn't
reach for `performWindowDragWithEvent:` and rediscover the same
failure mode.

---

## 2. Architecture

### 2.1 Component map

| Windows (existing, `HookMode::TabDrag`)                 | macOS (this plan)                                                          |
|--------------------------------------------------------|-----------------------------------------------------------------------------|
| `WH_MOUSE_LL` + `WH_KEYBOARD_LL` hooks, dedicated thread + `GetMessageW` pump | `CGEventTap` (mouse-moved + left-mouse-up + key-down event mask), dedicated thread + `CFRunLoopRun` |
| `WindowFromPoint` → `GetAncestor(GA_ROOT)`             | Point-in-rect test against `state.windows` bounds directly (see §2.3 — simpler than the parent spec's `windowNumberAtPoint:` prescription) |
| `SetWindowsHookExW` install / `UnhookWindowsHookEx` teardown | `CGEventTapCreate` + `CFMachPortCreateRunLoopSource` + `CFRunLoopAddSource` install / `CGEventTapEnable(false)` + `CFRunLoopRemoveSource` teardown |
| `ACTIVE_HOOK_THREAD` single-session guard              | Same pattern, reused verbatim (platform-agnostic `Mutex<Option<ThreadId>>`) |
| `PostThreadMessageW(WM_QUIT)` to stop                   | `CFRunLoopStop(loop)` (loop reference captured at spawn, sent across the ready-channel like Windows does) |
| IPC events: `tearoff:hover-changed`, `tearoff:hover-cleared` (continuous, drive the live insertion indicator — see §2.2 point 5), `tabdrag:merge-direct` (once, on mouse-up) | **Identical event names/payloads** — zero frontend changes (confirmed: `droppable-tab.tsx`'s `onDragStart` calls `startTabDragTracking` unconditionally today, on every platform, and just hits the no-op stub on macOS; `tab-tearoff-events.ts` already listens for all three unconditionally too) |

### 2.2 Threading & data flow

1. Frontend `onDragStart` calls `startTabDragTracking(...)` exactly
   as today (no frontend change) → IPC → `commands::drag::
   start_tab_drag_tracking` → routes to a new
   `#[cfg(target_os = "macos")] tear_off_hook::start_tab_drag_tracking`.
2. That function spawns a named thread (`"tear-off-hook-macos"`),
   mirroring Windows' `ready_tx`/`ready_rx` handshake so the IPC call
   doesn't return until the tap is confirmed installed *and enabled*
   (a `CGEventTapCreate` can succeed but return a disabled tap if
   Accessibility isn't granted — must be checked explicitly, see
   §2.4).
3. On that thread: create a `CFRunLoop` for the thread, install the
   event tap as a run-loop source, store the `CFRunLoopRef` +
   `CFMachPortRef` in thread-local storage (mirrors `HOOK_CTX`), run
   `CFRunLoopRun()`.
4. The tap callback runs **on the hook thread**, same as Windows'
   `low_level_mouse_proc` runs on the hook thread, not the CEF UI
   thread. It must not touch CEF/AppKit objects directly from here
   any more than the Windows code touches raw HWNDs without going
   through `state` — reuse the existing `state.list_browsers()` /
   `state.windows` snapshot-under-lock pattern verbatim. (This is
   also the exact class of bug flagged in
   `docs/investigations/tab-drag-tearoff-crash-macos.md` — a
   different, pre-CEF-migration incident, but the underlying lesson
   transfers directly: AppKit calls off the main thread are
   undefined behavior. The hook thread here must stick to CF-level
   point-in-rect math against already-cached bounds, never touch
   `NSWindow`/AppKit directly, and any state mutation goes back
   through the existing async IPC/event path — same discipline the
   Windows hook already follows.)
5. **Correction (caught before implementation):** Windows'
   `low_level_mouse_proc`/`handle_mouse_move` does **not** branch on
   `HookMode` at all — it emits `tearoff:hover-changed` /
   `tearoff:hover-cleared` on every mouse move regardless of mode,
   and `tab-tearoff-events.ts`'s listener for those two events
   doesn't check mode either — it's what drives
   `setInsertionPoint(computeInsertionPoint(clientX))`, the live
   insertion-point indicator, for `TabDrag` mode exactly the same as
   for the (now-dead) `TearOff` mode. So the macOS hook must emit
   **both**: `tearoff:hover-changed` (cursorX/Y in *physical* pixels,
   matching Windows — the frontend divides by `devicePixelRatio`
   itself) continuously while hovering a candidate window, and
   `tearoff:hover-cleared` when leaving one, **plus**
   `tabdrag:merge-direct` once on left-mouse-up if released over a
   candidate other than the source. Skipping the hover events would
   ship a version with no live preview during the drag — silently
   worse than intended, not just incomplete.
6. On left-mouse-up or Escape: same finalize/idempotency guard
   (`finalized: RefCell<bool>`) ported verbatim; `CFRunLoopStop`.

### 2.3 Hit-testing: simplification vs. the parent spec

The parent spec's §7 table prescribes `[NSWindow
windowNumberAtPoint:belowWindowWithWindowNumber:]` — an AppKit call
that resolves *any* window under the cursor system-wide, mirroring
`WindowFromPoint`'s generality. But Windows' own hit-test doesn't
actually need that generality either: `candidate_label_under_cursor_locked`
immediately throws away anything that isn't one of *our own* browser
windows (`is_instance_label` filter + matching against the
`browsers` snapshot). We already maintain `state.windows: HashMap<
String, cef::Window>` with `.bounds()` available on every entry.

**Decision:** skip `windowNumberAtPoint:` entirely. Do a plain
point-in-rect test against the small, already-in-memory list of our
own window bounds (typically single digits of windows). This is
strictly less code, avoids an extra ObjC round-trip per mouse-move
event (these fire at high frequency — every point-in-rect check
avoided is a real, if small, perf win), and sidesteps a subtlety
`windowNumberAtPoint:` has that `WindowFromPoint` doesn't: it
requires the *querying* app to own the window at `windowNumber`
being compared against, which complicates the "skip source window"
exclusion logic for no benefit here. Flag this as a deliberate,
reviewed deviation from the parent spec, not an oversight.

### 2.4 Accessibility permission (genuinely new subsystem)

Confirmed via investigation: **no existing TCC/Accessibility
permission code anywhere in this codebase.** (`macos_compat.rs`'s
accessibility governor is unrelated — it's Chromium's content-AX
tree for screen readers, not the macOS privacy permission.) This
needs to be built from scratch:

1. **Check, don't assume:** `AXIsProcessTrustedWithOptions(NULL)` —
   or with `kAXTrustedCheckOptionPrompt` set to `false` for a silent
   check — before attempting `CGEventTapCreate` at all.
   `CGEventTapCreate` doesn't fail loudly when unauthorized; it can
   return a tap that never fires, which would manifest as a silent,
   undebuggable "redock just doesn't work" bug if not checked
   explicitly up front. This must be a hard gate, not a try-and-see.
2. **First-use prompt:** on the first in-strip tab drag ever
   attempted, if untrusted: show an in-app explanation (new UI — a
   modal or toast, not the bare OS prompt alone, per the parent
   spec's own §6 parenthetical "with a clear UX explanation")
   *before* calling `AXIsProcessTrustedWithOptions` with
   `kAXTrustedCheckOptionPrompt: true` (triggers the OS's own System
   Settings-deep-link dialog). Persist "already asked" in app
   settings so we don't nag on every drag if the user declined.
3. **Graceful degradation when denied/not-yet-granted:** mirror the
   parent spec's own Wayland carve-out — in-strip drag still works
   exactly as it does on macOS *today* (the existing `DragOverlay`
   append-only fallback), just without the live insertion indicator
   / direct-merge upgrade. No new degraded-mode UX to invent; the
   fallback already exists and needs no changes.
4. **Settings deep link:** `NSWorkspace` open of
   `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`
   for users who want to grant it after initially declining
   (surfaced from wherever in-app settings/help lives).

### 2.5 Dependency decision (flag for review, not pre-decided)

Every existing macOS-native call in this codebase (`ui_tasks/
drag.rs`, `macos_compat.rs`, `ui_tasks/platform_macos.rs`) is raw
`extern "C"` FFI + `objc_msgSend` transmutes, with **zero** ObjC/
CoreGraphics/CoreFoundation crates in the dependency tree today.
`CGEventTapCreate`/`CFRunLoop*`/`AXIsProcessTrustedWithOptions` are
lower-level C APIs (ApplicationServices/CoreGraphics/CoreFoundation,
not AppKit `objc_msgSend` calls), so the "just add another raw
`extern "C"` block" idiom still applies mechanically, but this is
meatier, more state-carrying surface (run loop lifecycle, mach port
lifetime, CFDictionary construction for the trust-check options)
than the existing call sites, where a subtle retain/release or
run-loop-mode mistake is a plausible crash or silent-hang source.

Two options, both viable, genuine tradeoff:

- **(a) Continue the raw-FFI idiom** for consistency with the rest
  of the macOS code in this repo. Lower dependency-tree footprint,
  matches house style, but the team re-derives CF memory-management
  correctness by hand for run-loop/mach-port lifetime specifically.
- **(b) Add `core-foundation` + `core-graphics` crates**, scoped to
  `[target.'cfg(target_os = "macos")'.dependencies]` (a section that
  doesn't exist in `agentmux-cef/Cargo.toml` yet — would be new),
  for the CGEventTap/CFRunLoop/CFMachPort plumbing specifically,
  while leaving existing AppKit call sites (`NSWindow`, `NSEvent`,
  `NSApplication`) as raw FFI unchanged. These crates wrap exactly
  this surface with correct `Drop`-based retain/release, and are
  widely used/maintained (used by e.g. `rdev`, `keyboard-types`
  consumers for the same global-hotkey use case).

**Recommendation:** (b), scoped narrowly to the new CGEventTap/
CFRunLoop code only — since engineering cost isn't a constraint here
and correctness/robustness is explicitly the priority (parent spec
§0: "quality above all"), and CF memory-management bugs in a
long-lived background thread are the kind of defect that surfaces as
a rare, hard-to-repro crash days later. This is a call for whoever
picks up implementation to confirm, not a blocker to scoping.

---

## 3. What changes vs. what's already correct

**No changes needed (confirmed by investigation):**
- Frontend: `droppable-tab.tsx`, `tabbar-dnd.ts` — already
  platform-agnostic, already call `startTabDragTracking` and listen
  for `tabdrag:merge-direct` unconditionally on every platform.

**One small but necessary frontend change (caught before
implementation, corrects the "zero frontend changes" claim in an
earlier draft):** `tab-tearoff-events.ts`'s `physicalToClientX/Y`
unconditionally divides `payload.cursorX/Y` by
`window.devicePixelRatio` before subtracting `window.screenX/Y` —
correct for Windows (`GetCursorPos` reports raw physical pixels for
DPI-aware processes) but **wrong for macOS**: `CGEventTap`'s
`CGEvent.location()` reports coordinates in *points* — the same unit
`window.screenX/Y` and `getBoundingClientRect()` already use on
macOS, with no physical/logical distinction to convert (Retina scale
is baked into rendering, never exposed at this level). Emitting
already-in-points coordinates through the existing DPR-divide path
would silently shrink them by the display's backing scale factor on
any Retina screen. Fix: gate the division on `isWindows()` (already
imported/used elsewhere in this file's siblings) — divide by DPR on
Windows, pass through unchanged on macOS.
- IPC command registration shape (`ipc.rs`) — same command names,
  same `spawn_blocking` pattern, just routes to a macOS impl instead
  of the Windows one inside `commands::drag`.
- Event-name/payload contracts for `tearoff:hover-changed`,
  `tearoff:hover-cleared`, and `tabdrag:merge-direct` — all reused
  verbatim.
- Tear-off window placement — already at parity (§0.1); no changes.

**New/changed:**
- `agentmux-cef/src/commands/tear_off_hook.rs`: add
  `#[cfg(target_os = "macos")]` implementations of
  `start_tab_drag_tracking`, `stop_active_hook_session`,
  `candidate_label_under_cursor_locked` (macOS point-in-rect variant
  per §2.3), and the `TabDrag`-mode tap callback logic, replacing
  today's no-op stubs for that target only. (`start_tear_off_tracking`
  and any `TearOff`-mode logic are explicitly **not** ported — dead
  per §0.1. Linux/Wayland stubs untouched — out of scope.)
- New: Accessibility permission check/prompt/settings-deep-link
  subsystem (§2.4) — genuinely new code and (small) new UI, no
  existing pattern to lift from this codebase.
- `agentmux-cef/Cargo.toml`: new `[target.'cfg(target_os =
  "macos")'.dependencies]` section if §2.5(b) is chosen.

---

## 4. Phased delivery

Each phase independently mergeable and independently valuable.

**Phase 7a — Accessibility permission gate.** `AXIsProcessTrustedWithOptions`
silent check wired in ahead of `CGEventTapCreate`, with graceful no-op
fallback to today's `DragOverlay` behavior when untrusted.

**Revised during initial live testing:** a silent-only check made the
feature undiscoverable — with no in-app prompt (that's the rest of
7c) and no OS dialog either, a user with the permission ungranted saw
no visible difference from before and no way to grant it. Added a
minimal stand-in ahead of full 7c: silent check first, and if
untrusted, prompt with the OS dialog exactly once per process
lifetime (`PROMPTED_THIS_SESSION`, not persisted across launches —
that persistence is still 7c's job). Not the full UX (no in-app
explanation before the OS prompt, no settings deep-link surfaced
yet), but enough that a user can actually grant the permission and
try the gesture without editing System Settings by hand.

**Phase 7b — CGEventTap cross-window hit-test + `tabdrag:merge-direct`.**
The core deliverable: `HookMode::TabDrag`-equivalent hook, point-in-
rect hit-test (§2.3), emits `tabdrag:merge-direct` on mouse-up over
another AgentMux window — matching Windows' behavior exactly, same
event, same payload, zero frontend changes.

**Phase 7c — Accessibility permission UX polish.** First-use in-app
explanation modal, settings deep-link, "already asked" persistence.
Functionally 7b works without this (falls back silently per 7a); this
phase is the *user-facing* explanation/onboarding layer.

**Phase 7d — Validation & telemetry parity.** Reuse the parent
spec's §10 observability plan (`hook_install_failures`) for the
macOS tap, distinguishing genuine tap-creation failure from
expected-when-ungranted tap-disabled state (see §5.4). Build a macOS
equivalent of a repeated-drag stress test with assertions on leaked
threads / hook install failures.

---

## 5. Risks / open questions for whoever implements

1. **CGEventTap requires the process to be either the Accessibility-
   trusted app itself, or running as one with an inherited trust
   grant.** Since this app is launched via a separate launcher
   process fronting the CEF host, confirm *which* process needs the
   Accessibility grant — the launcher, or `agentmux-cef` itself —
   before building the permission UX around the wrong binary's
   identity. Needs a spike before 7b starts.
2. **Run loop mode:** confirm the tap's run-loop source is added in
   a mode that reliably fires during an active HTML5 drag session
   (the tab strip's own drag is driven by the renderer/WebKit, on a
   different thread than the hook) — no assumed interaction, verify
   empirically.
3. **Signed builds / entitlements:** Accessibility is a TCC
   permission, not an entitlement, so it generally needs nothing
   beyond existing code-signing — but confirm explicitly in Phase 7d
   rather than discovering an issue at notarization time.
4. **`hook_install_failures` counter** should distinguish
   "CGEventTapCreate returned null" (genuine API failure) from "tap
   created but disabled due to no Accessibility grant" (expected, not
   a bug) — conflating them would produce a permanently-nonzero
   counter for any user who hasn't granted the permission yet, which
   defeats its purpose as a regression signal.
5. **Known gap, confirmed via live testing:** the merge itself works
   end-to-end (`tabdrag:merge-direct` fires, the tab moves into the
   destination window), but the landing-bounce animation
   (`setBouncingTabId`, `tab-tearoff-events.ts`) doesn't reliably play.
   Likely cause: `setBouncingTabId(payload.tabId)` runs immediately
   after `await WorkspaceService.MoveTabToWorkspace(...)` /
   `RestoreTornOffTab(...)` resolves, but the destination window's
   `tabIds()`-driven tab list may not have re-rendered the merged
   tab's DOM element by that point yet (a possible second async hop
   between the RPC resolving and the reactive state update reaching
   this window) — `droppable-tab.tsx`'s `isBouncing` check would then
   never match anything, or match too late. Not investigated further
   at the user's call ("redock is in fact working, just not with full
   notification support") — left as a follow-up, not blocking.

- [ ] In-strip drag onto another AgentMux window's tab strip shows a
      live, cursor-accurate insertion indicator and merges directly
      on drop (`tabdrag:merge-direct`) — matching Windows'
      `TabDrag` mode exactly.
- [ ] Without Accessibility granted: in-strip cross-window drag still
      works via the existing `DragOverlay` append-only fallback, no
      crash, no hang, no silent failure.
- [ ] First drag attempt with no prior Accessibility grant shows the
      in-app explanation before the OS prompt; declining leaves the
      fallback behavior intact.
- [ ] "Already asked" persists — no repeat nagging on subsequent
      drags after a decline.
- [ ] `hook_install_failures` is 0 in steady state on a machine with
      Accessibility already granted.
- [ ] Repeated-drag stress equivalent passes with no leaked hook
      threads (confirm the `ACTIVE_HOOK_THREAD`-equivalent
      single-session guard actually prevents thread leaks under rapid
      repeated drags).
