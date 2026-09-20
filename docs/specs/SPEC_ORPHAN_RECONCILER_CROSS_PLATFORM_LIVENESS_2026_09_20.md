# SPEC: Real crash-orphan liveness check on macOS/Linux (closes #1569)

**Date:** 2026-09-20
**Status:** implemented — PR #3458.
**Related:** `docs/retro/wrr-design-2026-04-28.md` (why WRR is Windows-only),
`docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md` (origin spec for
`orphan_reconcile.rs`), `docs/specs/SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md`
§2.1 (the "sanitizer + executor, not a decider" design this fix stays
within), `docs/specs/SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` (a
Windows-only sibling bug in the parallel `win_event.rs` quit-trigger path
— not touched by this fix). GitHub issue #1569.

Note: issue #1569 cites an `ANALYSIS_WRR_CROSS_PLATFORM_2026_06_18` doc
under `docs/specs/` — no file by that name exists anywhere in the repo
(checked every `docs/` subtree). This spec cites `docs/retro/wrr-design-2026-04-28.md`
instead, which is the real source for the "WRR is Windows-only" claim.

---

## 0. The ask

On Windows, a renderer/window that dies without firing `on_before_close`
(a crash-orphan) is caught because WRR's native `WM_DESTROY` hook detects
the HWND vanishing and reclassifies it as `OrphanDestroy`, which re-emits
the same `HostShouldQuit` event a normal last-window-close would — waking
`orphan_reconcile.rs`'s reconciler. On macOS/Linux, nothing produces that
signal at all, and the reconciler's own per-browser liveness check is
hard-coded `Live` off Windows, so even if it DID run, it couldn't tell a
zombie browser from a live one. Fix both halves with a small, targeted
change — not a WRR port.

## 1. Current state (verified against the code, not the issue text)

### 1.1 `classify_hwnd` — the hard-coded half

`agentmux-cef/src/commands/orphan_reconcile.rs:434-461`:

```rust
fn classify_hwnd(browser: &Browser) -> HwndStatus {
    let b = browser.clone();
    let Some(host) = b.host() else { return HwndStatus::Hostless };
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        let wh = host.window_handle();
        if wh.0.is_null() { return HwndStatus::Dead; }
        if IsWindow(wh.0 as HWND) == 0 { HwndStatus::Dead } else { HwndStatus::Live }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = host;
        HwndStatus::Live
    }
}
```

`HwndStatus` (`:43-51`) is `Live | Hostless | Dead` — the input to the
pure, already-unit-tested planner `plan_reconcile` (`:107-169`), which
buckets every non-`browser-pane-*` browser label into
`zombie_closes`/`hostless_user`/`hostless_pool`/`freshly_promoted`. The
platform gap is entirely upstream of the planner, in how `HwndStatus`
gets produced — `plan_reconcile` itself needs no change.

The module's own doc comment already names this exact issue
(`orphan_reconcile.rs:227-234`): *"`classify_hwnd` hard-codes `Live` on
macOS/Linux (#1569), so the zombie/hostless buckets... are empty there."*
The Windows `IsWindow` arm shipped in #702.

### 1.2 `reconcile_and_drain` — the trigger half

`reconcile_and_drain(state: &Arc<AppState>)` (`:177-192`) doesn't do the
work directly — CEF Browser/BrowserHost calls must run on the CEF UI
thread, so it posts a `wrap_task!`-wrapped task to `ThreadId::UI`, whose
`execute()` calls `ui_thread_reconcile(&self.state)` (`:200-202`, the
function containing `classify_hwnd`'s call site).

**It is not on a timer.** Repo-wide, `reconcile_and_drain` has exactly
one call site: `agentmux-cef/src/launcher_ipc/mod.rs:720`, inside
`apply_event_to_shadow`'s `Event::HostShouldQuit` arm. `HostShouldQuit`
is emitted by the **launcher** reducer in two places:
- `agentmux-launcher/src/reducer/window.rs:273` — the normal
  last-window-closed path (every platform, always worked).
- `agentmux-launcher/src/wrr/mod.rs:339` — the crash/`OrphanDestroy`
  path, with the comment *"Without this, a crash-detected last-window
  close empties `state.windows` but never wakes the host's orphan
  reconciler (which only listens to `HostShouldQuit`)"* (`:317-319`).

**A precision the issue's "WRR is Windows-only by design" framing
slightly overstates:** `agentmux-launcher/src/wrr/mod.rs` (778 lines,
the reducer-side drift classification including the `OrphanDestroy`
handling above) has **zero** `#[cfg(target_os)]` gating — it compiles
and runs identically on every platform. What's actually Windows-only is
one layer further out: `agentmux-cef/src/wrr/mod.rs:24-43` (the CEF-host
native hook installer — `win_event.rs`'s `SetWinEventHook`, etc.) is
`#[cfg(target_os = "windows")]`-gated, with non-Windows stubs. That
installer is the **only** producer of the `Command::ReportHwndDestroyed`
that feeds the reducer's `OrphanDestroy` classification. So on
macOS/Linux the launcher-side reducer is present and capable, just
starved of input — there is no native signal to classify in the first
place. This spec's fix works around that gap from the CEF-host side
(§2.2) rather than building the missing native hook layer (explicitly
out of scope — see §4).

## 2. Proposed fix

### 2.1 Real liveness check: `Browser::is_valid()`

The issue's own snippet suggested "`has_view`/`is_valid`" as candidate
CEF APIs without picking one. Checked both, plus a third:

- **`cef::ImplBrowser::is_valid(&self) -> c_int`** — doc: *"True if this
  object is currently valid. This will return false (0) after
  `cef_life_span_handler_t::OnBeforeClose` is called."* Exactly the
  signal needed — callable directly on the `browser: &Browser` parameter
  `classify_hwnd` already has, no `unsafe`, no HWND/X11/NSView reasoning.
- `cef::ImplBrowserHost::has_view(&self) -> c_int` — despite the name
  matching the issue's suggestion, this means "is this browser wrapped
  in a `cef_browser_view_t`" (a Views-API question), not liveness. Wrong
  fit.
- `cef::ImplBrowserHost::is_ready_to_be_closed(&self) -> c_int` — about
  distinguishing cancelable vs. mandatory close-in-progress state from
  inside a close handler, not general liveness. Also wrong fit.

No existing call site in `agentmux-cef/src` uses `is_valid`/`has_view` —
this is a new, not a copied, call. Change:

```rust
#[cfg(not(target_os = "windows"))]
{
    let _ = host;
    if browser.is_valid() != 0 { HwndStatus::Live } else { HwndStatus::Dead }
}
```

Calling into a possibly-dead `Browser` is already proven safe on every
platform today: `classify_hwnd` unconditionally calls `b.host()` before
either `#[cfg]` arm (line 436, outside any platform gate), and that's
the production path already exercised on macOS/Linux — it returns `None`
(→ `Hostless`), never crashes. `is_valid()` is a plain read-only accessor
on the same object, not a destructive call, so it carries none of the
Views-controller-teardown-race hazard documented elsewhere in this
codebase (`browser_pane/creation_views.rs:597-605`,
`state/mod.rs:478-485` — about destroying a `cef::OverlayController`
mid-close, a different operation entirely).

### 2.2 New trigger: `on_render_process_terminated`

`agentmux-cef/src/client/crash_recovery.rs:25-413`,
`impl AgentMuxHandler { pub(crate) fn on_render_process_terminated(&mut self, browser: Option<&mut Browser>, status: TerminationStatus, error_code: i32, error_string: Option<&CefString>) }`
— wired to the CEF vtable at `client/handlers.rs:620-651`. (The issue's
citations of `handlers.rs:396`/`client/mod.rs:1491` are stale — this
function was extracted into its own file at some point since #1569 was
filed.) Today it does white-screen crash recovery only: rate-limited
logging, a gated memory-pause recovery page, a per-browser crash-budget
check, then loads an HTML recovery page into the dead browser's frame.
**It never calls `reconcile_and_drain` or anything orphan-related today**
— confirmed the only call site anywhere is §1.2's launcher-event arm, so
this is a new wire-up, not a rewire.

Add, gated to non-Windows (Windows already has a working, independently
verified trigger path via WRR's native hook — see §1.2 — so this fix
only closes the macOS/Linux gap rather than touching the working path):

```rust
#[cfg(not(target_os = "windows"))]
crate::commands::orphan_reconcile::reconcile_and_drain(&self.state);
```

`self.state: Arc<AppState>` is already a field on `AgentMuxHandler`
(`client/mod.rs:169`) and already used elsewhere in this same file
(e.g. `crash_recovery.rs:226`), so no new plumbing is needed.

**Why this doesn't risk over-triggering on a benign exit:** CEF's own
contract for this callback (`cef_termination_status_t`) rules that out —
the doc says it's *"called... when the render process terminates
**unexpectedly**"*, and `TerminationStatus` has no "clean exit" variant
at all: only `ABNORMAL_TERMINATION`, `PROCESS_WAS_KILLED`,
`PROCESS_CRASHED`, `PROCESS_OOM`, `LAUNCH_FAILED`, `INTEGRITY_FAILURE`. A
normal renderer shutdown never reaches this handler — that's
`on_before_close`, wired separately. The new call should tolerate
`browser == None` (the handler already defensively handles that
elsewhere in the file, e.g. `:168-169`, `:192`) — `reconcile_and_drain`
doesn't take a specific browser anyway, it re-derives state from the
whole reducer snapshot, so a `None` browser here changes nothing about
whether to call it.

## 3. What this does NOT prove (scope the claim honestly)

`Browser::is_valid()` only proves the CEF-level C++ object hasn't been
torn down (i.e. `OnBeforeClose` hasn't fired) — it does **not** prove the
renderer is responsive. `agentmux-cef/src/state/promote_liveness.rs:1-40`
already documents (citing
`docs/retro/retro-fresh-vm-suspend-orphaned-frontend-2026-09-03.md`) that
neither `IsWindow()` nor `window_handle()` proves a renderer is alive —
both survive a suspend/resume that left the page dead. `is_valid()` has
the same *kind* of limitation: it catches the crash-orphan case this
issue is about (the CEF object itself is gone), not a hung-but-technically-
alive renderer. That's an existing, separate class of problem, not a
regression this fix introduces.

## 4. Explicitly out of scope

- **A full macOS/Linux WRR observation layer** (`OffMonitor`/
  `HiddenSinceOpen`/`LingeringHwnd`-equivalent native hooks). Per
  `docs/retro/wrr-design-2026-04-28.md`, WRR's entire mechanism is built
  on Win32-specific hooks (`SetWinEventHook`, `WM_WINDOWPOSCHANGED`) with
  no X11/Cocoa equivalent ever designed — low value, high cost, and
  infeasible under Wayland (no equivalent enumeration API). This fix
  closes the **crash-orphan detection** gap specifically, via a
  CEF-level signal that needs no native window-event hook at all — it
  does not attempt parity with WRR's fuller drift-classification set.
- **The Windows path.** Not modified. `classify_hwnd`'s Windows arm and
  the WRR-driven trigger chain are unchanged; only the non-Windows arms
  of both functions gain new behavior.
- **Native window bugs** named in issue #2189 (tear-off ghost-pane,
  redock hit-test, Wayland pool visibility) — separate, native-code
  work, unrelated to this liveness-detection gap.

## 5. Platform scope

Both changes are entirely inside existing `#[cfg(not(target_os =
"windows"))]` arms (§2.1) or newly added ones (§2.2) — no behavior change
on Windows. Applies identically to macOS and Linux (`is_valid()` is a
CEF-level API, not X11/Cocoa-specific), consistent with how
`classify_hwnd`'s existing non-Windows arm already treats macOS and
Linux as one case.

## 6. Test plan

- `plan_reconcile` is already pure and unit-tested (`orphan_reconcile.rs:530-836`)
  against `HwndStatus` values directly — no change needed there, and no
  new unit test needed for the bucketing logic itself.
- `classify_hwnd`'s non-Windows arm cannot be unit-tested without a live
  CEF `Browser` (same reason `orphan_reconcile.rs` has no existing test
  for the Windows arm's `IsWindow` call either — it's an FFI boundary).
  Verify via live manual test on Linux: launch, force-kill a renderer
  process (`kill -9` the renderer PID directly, bypassing CEF's own
  close path — mirrors how a genuine crash looks from the host's
  perspective) with more than one window open, confirm
  `on_render_process_terminated` fires, `reconcile_and_drain` runs (log
  line), and the dead browser's label lands in `zombie_closes` rather
  than being treated as `Live`.
- Regression: close a window normally (not a crash) on Linux and confirm
  no double-reconciliation weirdness — `on_before_close`'s existing path
  is untouched by this fix, and `on_render_process_terminated` should
  simply never fire for that case (§2.2's CEF-contract argument), so
  this is really a confirm-no-regression check rather than new behavior
  to verify.

## 7. Open follow-ups

- Whether to also add the `on_render_process_terminated` trigger on
  Windows (redundant with the existing WRR-driven path, but
  `reconcile_and_drain` is designed to be safe to call repeatedly per
  the "sanitizer + executor" model) — deliberately deferred rather than
  bundled into a macOS/Linux bug-fix PR; a follow-up if there's ever a
  concrete case where WRR's `WM_DESTROY` path alone proves insufficient.
