# Windows: widget "More" menu / hamburger menu render over panes but don't accept hover/click

**Date:** 2026-07-13 · **Author:** Agent1 · **Trigger:** user report — AgentO's
flyout-menu-over-browser-pane work (verified working on macOS) renders
correctly on the `local-main-b28b7a-577e65c6` Windows instance, but hovering
over an item in the widget bar's "More" dropdown or the hamburger menu does
nothing, and clicking an item is a no-op, whenever the menu is positioned
over a browser or messenger pane.

> **Verdict (one line):** Almost certainly a **hit-testing gap, not a
> rendering gap** — `SetWindowRgn` (the Win32 "punch a hole so DOM shows
> through a native pane" mechanism) is applied only to CEF's own pane HWND,
> never to the **app-owned wrapper HWND** introduced five weeks later
> (2026-07-03, PR #1957) for an unrelated bug. The wrapper fully covers the
> same rect, was never given a matching region, and silently swallows every
> mouse message in the "hole." This is Windows-only and would not reproduce
> on macOS/Linux, which don't have this wrapper (or `SetWindowRgn`) at all.

---

## 1. Reproduction context

Checked the reported instance's own logs (it isn't currently running, so
this is post-hoc log analysis, not a live CDP session):

```
C:\Users\asafe\.agentmux\channels\local-main-b28b7a-577e65c6\versions\0.53.4\logs\agentmux-host-v0.53.4.log.2026-07-14
```

- 1× `[pane-wrapper] created` — confirms this session did create at least
  one embedded browser pane on Windows, going through the app-owned wrapper
  HWND path described below.
- 15× `[pane-airspace] applied overlay clip to pane HWNDs` with
  `overlay_count` toggling between 0/2/4 — confirms the overlay-clip
  mechanism *was* firing repeatedly, consistent with a menu being
  opened/closed/hovered several times while a pane was open. So the
  frontend→backend clip pipeline is running end-to-end; the failure is not
  "the mechanism never engaged," it's "the mechanism engages but only half
  of what needs clipping gets clipped" (see §4).

No crash, panic, or error-level log line near these events — this is a
silent behavioral gap, not a fault.

## 2. Background: how the airspace hole-punch works (unchanged since May)

Fully described in `docs/analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`
(read in full before this doc); short version:

On Windows, an embedded browser pane is a **native child HWND**, which always
composites *above* DOM regardless of CSS z-index (the "airspace" problem).
Any DOM element that must show through a pane tags itself `data-pane-overlay`
+ calls `usePaneOverlay()` (`frontend/app/platform/pane-overlay.ts`). Both
`FlyoutMenu` (hamburger + its submenus, `frontend/app/element/flyoutmenu.tsx:253,378`)
and the widget bar's `MoreDropdown` (`frontend/app/window/action-widgets.tsx:159,221`)
do this — **same primitive, same code path**, which is why the user sees the
identical symptom on both surfaces; this is one bug, not two.

The rect is sent to the host over `browser_panes_set_overlay_clip`. The
Rust handler, `BrowserPanes::set_pane_overlay_clip`
(`agentmux-cef/src/browser_panes.rs:659-880`, Windows branch), calls Win32
`SetWindowRgn` on the pane's HWND to **subtract** the overlay rect from the
pane's own paintable/hit-testable region. Outside the hole, the pane covers
the DOM as before; inside it, the pane no longer claims that area, so
whatever's underneath is what the user sees *and interacts with* — normally.

## 3. What changed five weeks later: the app-owned wrapper HWND

`docs/specs/SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md` (2026-07-03)
and its implementation, PR #1957 (`fix(browser-pane): app-owned wrapper HWND
fixes Windows renderer leak on pane close`), inserted a **new HWND** into the
parent chain, for a completely unrelated bug (closing a pane never tore down
its CEF renderer process — issue #1936). The fix: an app-owned, app-defined
window class (`AgentMuxPaneWrapper[-hash]`, `agentmux-cef/src/browser_pane/wrapper.rs`)
now sits **between** the pane's host window and CEF's own browser HWND:

```
main (or wherever the pane lives)
 └─ wrapper HWND   (WS_CHILD, our own class, agentmux-cef/src/browser_pane/wrapper.rs)
     └─ pane HWND  (WS_CHILD, CEF's own — this is what host.window_handle() returns)
```

The wrapper fills exactly the pane's rect; CEF's own HWND fills the
wrapper's entire client area as its sole child (`wrapper_wndproc`'s `WM_SIZE`
handler keeps them congruent — `wrapper.rs:98-114`). The wrapper's window
class registers with **`hbrBackground: null`** (`wrapper.rs:159`, "the CEF
child fills the client area before any WM_ERASEBKGND would matter") and
handles only `WM_SIZE`/`WM_DESTROY` — every other message, including every
mouse message, falls through to `DefWindowProcW` untouched (`wrapper.rs:126-129`).

**`set_pane_overlay_clip` was never updated to account for this new window.**
It resolves the pane via `browser.host()` → CEF's `BrowserHost::window_handle()`
(`browser_panes.rs:709`) — which is, and has always been, CEF's *own* inner
HWND — and calls `SetWindowRgn` on **that**, exclusively
(`browser_panes.rs:752`, `:853`). The wrapper's region is never touched
anywhere in this function or in `wrapper.rs`. `git log` on `browser_panes.rs`
since the wrapper landed shows exactly one touch (#2098, the macOS flyout
fix, unrelated) — the region-punch logic itself hasn't been revisited since
before the wrapper existed.

One detail worth flagging precisely: the author of `set_pane_overlay_clip`
clearly *knew* the wrapper existed by the time this code was last touched —
the function already walks `GA_ROOT` instead of a single `GetParent` hop
specifically because "`pane_hwnd` is a `WS_CHILD` of our own wrapper HWND,
not directly of main" (`browser_panes.rs:774-783`, comment). So this isn't a
case of the wrapper being invisible to this code — the *positioning* math was
correctly adjusted for it, but the *region* was not also propagated to it.
An easy thing to miss: `GetAncestor(..., GA_ROOT)` silently walks past the
wrapper for the "which top-level window is this" question, which reads as
"the wrapper is handled" — but `SetWindowRgn` doesn't walk anything; it has
to be called on each HWND individually, and the wrapper's own call was never
added.

## 4. Why this produces exactly "renders, but hover/click no-op" — not a visual glitch too

This is the detail that makes the symptom look confusing at first (why isn't
there *also* a black spot or offset, per the older DPI-mismatch bug class?)
but it falls out cleanly once painting and hit-testing are considered
separately, which is exactly how Win32 treats them:

- **Painting:** the wrapper's class has `hbrBackground: null` and no
  `WM_PAINT`/`WM_ERASEBKGND` handler. A window that paints *nothing itself*
  simply leaves whatever was already composited underneath at the DWM/GDI
  level, at that surface, untouched — no `WS_EX_TRANSPARENT` or
  `WS_EX_LAYERED` needed for that. So once `SetWindowRgn` excludes the
  overlay rect from *CEF's own HWND* (the innermost window, which is the
  only one that ever actually draws anything there), nothing in the whole
  wrapper→pane chain paints over that pixel region — the DOM menu underneath
  shows through correctly. This matches the user's "it renders."
- **Hit-testing:** Win32 mouse hit-testing walks the child-window tree by
  **rectangle-or-region**, independent of what actually got painted. A
  `SetWindowRgn`-excluded point on the pane HWND makes Windows skip *that
  HWND* as a candidate — but the search doesn't fall through past the pane's
  own immediate parent. It resolves to the **wrapper**, because the wrapper
  still claims its full, un-clipped rectangular region there (no
  `SetWindowRgn` call was ever made on it). The wrapper has no mouse-message
  handling of its own, so `WM_LBUTTONDOWN`/`WM_MOUSEMOVE` land on
  `DefWindowProcW` and are silently absorbed — no click fires, no `:hover`
  reaches the DOM (the OS never delivered the input event to the browser
  process hosting that DOM at all). This matches "hover not working, click
  is a no-op" precisely, including that it's a *silent* no-op with nothing
  in the console — there's nothing to catch, because the DOM's event
  listeners are never invoked in the first place.

## 5. Why macOS was clean (and why AgentO's verification didn't catch this)

macOS/Linux don't use child HWNDs for panes at all — `docs/analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`
§1: panes there are `CefBrowserView`s, and the "hide the whole pane instead
of punching a hole" workaround is architecturally different (no
`SetWindowRgn` equivalent, no wrapper window, no region-vs-rectangle
hit-testing split — see `browser_panes.rs:883-` for the non-Windows path).
The wrapper HWND is behind `#![cfg(target_os = "windows")]`
(`wrapper.rs:41`) and doesn't exist as a concept on the other platforms.
AgentO's PR #2098 (macOS flyout-over-pane occlusion/click-routing fix) and
its live verification were entirely correct for that platform — this is a
distinct, Windows-specific gap that opened up independently, five weeks
after the airspace mechanism was last validated there, as a side effect of
an unrelated Windows-only bug fix.

## 6. Confidence and what would nail it down further

**High confidence** in the mechanism (§3-4) from static code reading — the
absence of any `SetWindowRgn` call on `wrapper_hwnd` anywhere in the
codebase is a clean, structural fact, not an inference, and it fully
explains every part of the reported symptom (renders / no hover / no click /
silent / Windows-only / post-dates the working macOS verification). What
this analysis has **not** done, and what would make it certain rather than
"almost certainly":

1. **Live repro with Spy++ / Window Detective** — hover over a More-dropdown
   item positioned over a pane, confirm the HWND actually receiving
   `WM_MOUSEMOVE` at that screen point is the `AgentMuxPaneWrapper[-hash]`
   class, not the DOM/main window. This is the single most direct
   confirmation available and doesn't require a code change.
2. Alternatively, a temporary `tracing::debug!` in `wrapper_wndproc`'s
   fallthrough arm, logging any mouse message it receives with its
   screen-point — reproduce, then check whether it fires while hovering a
   "dead" menu item.

## 7. Proposed fix (not yet implemented — flagging shape, not shipping)

Mechanically small: apply the **same region**, in the **same wrapper-local
coordinates** (the wrapper and CEF's own HWND are kept congruent — same
origin, same size — by `wrapper_wndproc`'s `WM_SIZE` handler, so no new
coordinate math is needed) to `wrapper_hwnd` right alongside the existing
`SetWindowRgn(pane_hwnd, ...)` calls in both branches of
`set_pane_overlay_clip` (`browser_panes.rs:752` restore-to-full-visibility,
and `:853` apply-clip). `peek_wrapper_hwnd(label)` (`wrapper.rs:66-68`,
already used elsewhere in this same file for resize) is the lookup this
would use. `CreateRectRgn`/`CombineRgn` already build one region object per
call — either apply it to both HWNDs (regions are not shared handles once
`SetWindowRgn` takes ownership, so a second, separately-created region would
be needed for the wrapper) or restructure slightly to build it once and use
`SetWindowRgn`'s "system takes ownership" semantics correctly for two
targets. Left as a follow-up, not attempted here per the ask to investigate
and report first.

---

### File references

- `agentmux-cef/src/browser_panes.rs` — `set_pane_overlay_clip` (`:659-880`,
  Windows branch); the two `SetWindowRgn(pane_hwnd, ...)` call sites
  (`:752`, `:853`) that would need a wrapper-HWND counterpart
- `agentmux-cef/src/browser_pane/wrapper.rs` — wrapper HWND module; class
  registration (`hbrBackground: null`, `:159`), `wrapper_wndproc` (`:81-130`,
  only handles `WM_SIZE`/`WM_DESTROY`), `peek_wrapper_hwnd` (`:66-68`)
- `docs/specs/SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md` — why
  the wrapper exists (renderer-teardown leak, unrelated to airspace)
- `docs/analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md` —
  the airspace/overlay-clip mechanism this gap sits inside; predates the
  wrapper by 5 weeks
- `frontend/app/platform/pane-overlay.ts` — `usePaneOverlay`, frontend side
  of the clip pipeline (unaffected — the frontend's contract is honored
  correctly; the gap is entirely on the Rust/Win32 side)
- `frontend/app/element/flyoutmenu.tsx` (`:253`, `:378`) and
  `frontend/app/window/action-widgets.tsx` (`:159`, `:221`) — the two
  reported-affected surfaces, both going through the identical
  `usePaneOverlay` primitive
- Instance logs analyzed: `C:\Users\asafe\.agentmux\channels\local-main-b28b7a-577e65c6\versions\0.53.4\logs\agentmux-host-v0.53.4.log.2026-07-14`
