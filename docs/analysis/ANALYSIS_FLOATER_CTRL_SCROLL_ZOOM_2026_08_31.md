# Analysis: Ctrl+Scroll zoom does not work in a torn-off floating pane

**Status:** **Test A has been run and the §2 hypothesis is DISPROVEN.** See §3.1.
The live candidate is now §4. Do not implement §5 as written — its premise is gone.
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

That is the *entire* affirmative case, and it is weak — it describes the initial
show only, not the steady state.

### What does NOT support it, though an earlier draft claimed it did

An earlier version of this document argued that the **absence** of
`WM_MOUSEACTIVATE` and `SetFocus` from `floating_pane.rs` was evidence the CEF
child never gets focus. **That reasoning is invalid** and is retracted here:

- `floating_pane_wndproc` ends in `DefWindowProcW` (`floating_pane.rs:703`), so
  every message it does not explicitly handle gets **default** Win32 processing —
  including the ordinary click-to-activate and focus path.
- The browser is embedded as a real child HWND (`floating_pane.rs:261`,
  `WindowInfo::set_as_child`), so normal Win32/CEF focus handling applies to it.

Absence of explicit focus code is therefore not evidence of absent focus; it is
evidence that focus is left to the default, which very likely works. Recorded
because the mistake is easy to repeat when reading this file.

### What cuts against it

- Tear-off calls `SetForegroundWindow(dest_hwnd)` (`commands/drag.rs:651`), so the
  floater's outer popup does come forward.

Taken together, the hypothesis is now only weakly supported and has two independent
strikes against it. It is still worth testing because it is cheap to test — not
because the source reading favours it. Get the measurement.

## 3. The two tests that settle it

**Test A — does the renderer have keyboard focus at all?**
Tear off an **agent or terminal** pane and type into it.

- Typing works → the child has keyboard focus, `ctrlKey` should be true, and this
  whole hypothesis is **dead**. Go to §4's alternative.
- Typing does not work → hypothesis **confirmed**, and the bug is much bigger than
  zoom (a torn-off pane would be keyboard-dead).

### 3.1 Test A result (2026-08-31) — hypothesis disproven

Repo owner ran it: *"I typed into a floating terminal .. typing works but not zoom."*

Typing works, so the floater's CEF child **does** have keyboard focus, its renderer
**does** receive the Ctrl keydown, and `ev.ctrlKey` is therefore tracked correctly.
The §2 mechanism is dead. This is the outcome Test A was written to produce, and it
cost one keystroke instead of a speculative Win32 focus fix — the reason the fix was
deliberately not built first.

Note this also retires §5 as written: with `ev.ctrlKey` known-good, there is no
longer any reason to bypass the DOM via `MK_CONTROL`. Whatever is wrong is either
upstream of the DOM event or downstream in the zoom pipeline, and §5 addresses
neither.

**Test B is now moot** (it was a refinement of the dead hypothesis) and is kept only
for the record. The next test is §4.1.

**Test B — is it focus-order dependent? (SUPERSEDED — see §3.1)**
In the floater: click the main window, then hover the floater and ctrl+scroll.
Then click *inside* the floater once and ctrl+scroll again.

- Only the second works → confirmed, and it is specifically an activation-order bug.
- Neither works → focus is not the variable.

## 4. The live candidates (post-Test-A)

With `ev.ctrlKey` established as trustworthy, the failure is on one of two sides of
the DOM event, and they need to be separated before anything is built:

**4a. Upstream — the wheel event never reaches the renderer with Ctrl set.** CEF
consuming Ctrl+Scroll for its native page-zoom path before the DOM sees it, so
`preventDefault()` never runs. Note the only host-side `WM_MOUSEWHEEL` interception
in the tree (`browser_pane/hwnd.rs:403`) is scoped to browser panes, not floaters,
so if this is the mechanism it is inside CEF rather than our own code.

**4b. Downstream — the handler runs but the zoom pipeline no-ops.** For a terminal
the path is `AppZoomHandler` → `target.closest("[data-blockid]")` →
`zoomBlockIn/Out` → `getBlockZoom` → `getBlockComponentModel(blockId)`. That last
call returns `null` when the block isn't in the module-level registry
(`store/block-component-registry.ts:13`), and the caller then bails **silently** —
which matches "nothing happens at all" exactly.

Reading the source does not separate these: registration at `block.tsx:291` looks
unconditional on mount, and the floater renders the standard `<TabContent>`, so 4b
*should* work. It needs a measurement.

### 4.1 The test that separates them

In the floater window, open DevTools (hamburger ▸ DevTools, or View ▸ Toggle
DevTools on macOS) and run:

```js
window.addEventListener('wheel', e => console.log('wheel ctrl=', e.ctrlKey, 'dy=', e.deltaY), { passive: false, capture: true });
```

Then Ctrl+Scroll over the pane body.

| Observation | Conclusion |
|---|---|
| nothing logs | **4a** — the event is consumed before the renderer |
| logs with `ctrl= false` | **4a** — modifier lost upstream (contradicts Test A; re-test typing) |
| logs with `ctrl= true` | **4b** — the DOM is fine; the bug is in the zoom pipeline |

If it is 4b, the follow-up is whether `[data-blockid]` resolves — in the same
console:

```js
document.querySelectorAll('[data-blockid]').length
```

Zero means `AppZoomHandler`'s `closest()` lookup is what fails; non-zero points at
the registry lookup inside `getBlockZoom`.

## 5. Fix shape (do not build until §3 confirms)

The idea is the one already proven for browser panes: intercept `WM_MOUSEWHEEL`,
test `wparam & 0x0008` (`MK_CONTROL`), and drive the existing zoom pipeline from
there — bypassing the DOM's `ctrlKey` entirely, since `MK_CONTROL` is
focus-independent.

**Where to hook is the load-bearing detail, and hooking `floating_pane_wndproc`
alone is NOT sufficient.** The browser is a child HWND, and Chromium creates its
own descendant HWNDs beneath it. A wheel message delivered to a descendant is
handled there and the outer popup's WndProc may never observe it — which is exactly
the case under §4's alternative mechanism. The existing implementation reflects
this: `install_browser_pane_focus_redirect` (`browser_pane/hwnd.rs:312`) subclasses
the outer HWND **and every descendant Chromium has already created**, and
`WM_MOUSEWHEEL` is handled in that hook (`hwnd.rs:404`), not in any outer wndproc.

That hook also has to be **re-applied after every navigation** — Chromium recreates
`Chrome_RenderWidgetHostHWND` on each page load, so a subclass installed once ends
up stranded on a destroyed HWND (see the doc comment at `browser_pane/hwnd.rs:312`,
and its wiring from `on_after_created_browser_pane` *and* `on_load_end_browser_pane`).

So the fix must hook the embedded browser hierarchy the same way, or use another
guaranteed input path. Until it does, it cannot be described as correct under either
mechanism.

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
