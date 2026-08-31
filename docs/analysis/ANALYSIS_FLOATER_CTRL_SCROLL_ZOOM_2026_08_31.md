# Analysis: Ctrl+Scroll zoom does not work in a torn-off floating pane

**Status:** Diagnosis narrowed to one mechanism, **not yet empirically confirmed**.
Two cheap tests below discriminate it. Do not implement a fix before running them.
**Date:** 2026-08-31
**Author:** AgentA
**Reported by:** repo owner — *"ctrl+scroll zooming stops working once the pane is
torn off"* (observed on a torn-off Armory pane).

---

## 1. What is ruled OUT

These were the obvious candidates. All three are dead, verified in source:

| Candidate | Why it's out |
|---|---|
| Handler not mounted in the floater | `AppZoomHandler` (`frontend/app/app.tsx:189`) is rendered at `app.tsx:355`, **above** the `<Show when={!IS_FLOATING_PANE}>` gate at line 359. It runs in floater windows. |
| Armory's own handler missing | `armory-view.tsx:64` attaches a capture-phase `wheel` listener on `viewRef` and calls `stopPropagation()`, so it wins over the global one — identically in both docked and floating cases. |
| `[data-blockid]` absent from the floater DOM | `floating-pane-workspace.tsx:1046` renders the standard `<TabContent>`, which renders the standard `BlockFrame`, which emits `data-blockid` at `blockframe.tsx:951`. No floater-specific gate. |
| Meta write not round-tripping | Write (`SetMetaCommand` on `block:<id>`) and read (`armory-model.ts:87`, `blockAtom().meta["term:zoom"]`) use the same object. The floater's auto-close logic depends on receiving Tab object broadcasts and works, proving broadcasts reach floater windows. |

## 2. The remaining mechanism

The DOM's `ev.ctrlKey` and the Win32 `MK_CONTROL` bit come from **different sources**,
and only one of them is focus-dependent:

- `WM_MOUSEWHEEL`'s `wParam & MK_CONTROL` is set by the system from **global** key
  state. It does not care which window has keyboard focus. This is what
  `browser_pane/hwnd.rs:404` reads, and why per-pane browser zoom works.
- The renderer's `ev.ctrlKey` comes from CEF's **per-renderer tracked modifier
  state**, which is built from keyboard events. A browser that never receives the
  Ctrl keydown reports `ctrlKey === false`.

Mouse messages route by cursor position; keyboard messages route by focus. So a
window can receive the wheel while reporting `ctrlKey: false` — which presents
exactly as "ctrl+scroll behaves like plain scroll", i.e. the reported symptom.

**Hypothesis:** the floater's inner CEF browser child never gets keyboard focus, so
its renderer's Ctrl state is permanently false.

### What supports it

- The floater is created `SW_SHOWNOACTIVATE` (`floating_pane.rs:854`).
- `floating_pane_wndproc` handles `WM_NCCALCSIZE`, `WM_NCACTIVATE`, `WM_NCHITTEST`
  and `WM_SIZE` — there is **no `WM_MOUSEACTIVATE` and no `SetFocus`** anywhere in
  `floating_pane.rs`.
- The one focus-restore path (`client/wndproc.rs:66`, on `WM_ACTIVATE`) restores to
  `LAST_FOCUSED_BY_ROOT` — a map keyed on *browser panes*. Whether a floater's child
  is ever registered there is unverified.

### What cuts against it

- Tear-off **does** call `SetForegroundWindow(dest_hwnd)` (`commands/drag.rs:651`),
  so the floater's outer popup comes forward. If that also lands keyboard focus on
  the CEF child, the hypothesis is dead.

That contradiction is precisely why this needs a measurement rather than another
source reading.

## 3. The two tests that settle it

**Test A — does the renderer have keyboard focus at all?**
Tear off an **agent or terminal** pane and type into it.

- Typing works → the child has keyboard focus, `ctrlKey` should be true, and this
  whole hypothesis is **dead**. Go to §4's alternative.
- Typing does not work → hypothesis **confirmed**, and the bug is much bigger than
  zoom (a torn-off pane would be keyboard-dead).

**Test B — is it focus-order dependent?**
In the floater: click the main window, then hover the floater and ctrl+scroll.
Then click *inside* the floater once and ctrl+scroll again.

- Only the second works → confirmed, and it is specifically an activation-order bug.
- Neither works → focus is not the variable.

## 4. Alternative if the tests kill the hypothesis

CEF is consuming Ctrl+Scroll before the DOM in the floater window (its native
page-zoom path), so `preventDefault()` never gets the chance to run. The docked case
would differ only if the main window's browser is configured to suppress it.

## 5. Fix shape (do not build until §3 confirms)

The robust fix is the same one already proven for browser panes: intercept
`WM_MOUSEWHEEL` in `floating_pane_wndproc`, test `wparam & 0x0008` (`MK_CONTROL`),
and drive the existing zoom pipeline from there — bypassing the DOM's `ctrlKey`
entirely. It is correct under **either** remaining mechanism, because `MK_CONTROL`
is focus-independent.

Unlike the browser-pane case, the floater cannot reuse `browser_panes::zoom_in`
(that applies a CSS zoom to a browser pane's own document). A floater wraps exactly
one block of arbitrary type, so the host must instead signal the floater's frontend
to run its normal `term:zoom` meta write.

**Note the platform gap this inherits:** `browser_pane/hwnd.rs`'s equivalent
interception is `#![cfg(target_os = "windows")]`-gated and has no macOS/Linux
counterpart (documented as a known limitation at `hwnd.rs:395`). A floater fix built
this way would carry the same gap and must say so rather than appear cross-platform.

## 6. Not covered by the existing report

`docs/specs/REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md` covers
Armory zoom and per-pane browser zoom but says nothing about floating panes or
window focus. This is a new finding, not a regression of that work.
