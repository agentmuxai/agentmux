# Analysis: Ctrl+Scroll zoom does not work in a torn-off floating pane

**Status:** **Root cause localised by direct measurement — see §6.** The entire
in-page pipeline (DOM event → handler → RPC → read-back → font) is proven WORKING
in a live floater. The failure is upstream of the renderer, in the OS→CEF input
path. §2's focus hypothesis is dead; §4b is dead; §4a survives.
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

Typing works, so the floater's CEF child **does** have keyboard focus for ordinary
character input. The §2 mechanism — "the child never gets focus at all" — is dead.
This is the outcome Test A was written to produce, and it cost one keystroke instead
of a speculative Win32 focus fix, which is why the fix was deliberately not built
first.

**Correction (Codex P2 on the PR adding this section):** an earlier draft of §3.1
went further and declared `ev.ctrlKey` "known-good" on this evidence alone. That was
an overclaim. Typing proves character input reaches the renderer; it does **not**
prove that the Ctrl *modifier* is attached to a wheel event, which is a different
delivery path. The narrower hypothesis — focus is fine, but the modifier or the
wheel message itself is lost upstream — survived Test A untouched, and §6 shows it
is in fact the live one. The overclaim would have wrongly retired §5 outright;
§5 is instead **back on the table** (see §6.2).

**Test B is now moot** (it was a refinement of the dead hypothesis) and is kept only
for the record. The next test is §4.1.

**Test B — is it focus-order dependent? (SUPERSEDED — see §3.1)**
In the floater: click the main window, then hover the floater and ctrl+scroll.
Then click *inside* the floater once and ctrl+scroll again.

- Only the second works → confirmed, and it is specifically an activation-order bug.
- Neither works → focus is not the variable.

## 4. The candidates after Test A

> **Resolved by §6: 4b is dead, 4a survives.** Kept as written because the reasoning
> for *why* both were live is what §6's experiment was designed around.

With keyboard focus ruled in, the failure is on one of two sides of the DOM event,
and they need to be separated before anything is built:

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

### 4.1 How to probe a floater — CDP, not DevTools

**A floater has no DevTools entry point of its own.** It renders the chromeless
`FloatingPaneWorkspace` (no tab bar, no widgets bar, no hamburger), and the native
View ▸ Toggle DevTools menu is macOS-only. The pane context menu's *Inspect Element*
(`block/pane-actions.ts:203`) is the one in-app route on Windows/Linux.

The route that actually worked, and that needs no UI interaction at all:

```bash
curl -s http://127.0.0.1:9223/json/list        # 9223 dev / 9222 release
```

Each floater is its own page target, identifiable by `windowLabel=floating-…` in
its URL, with a `webSocketDebuggerUrl`. Drive it over CDP (`Runtime.evaluate` to
instrument, `Input.dispatchMouseEvent` with `type: "mouseWheel", modifiers: 2` to
inject Ctrl+Wheel, `Page.captureScreenshot` to see the result).

**Caveat that determines what this can and cannot prove:** `Input.dispatchMouseEvent`
enters *below* the OS input layer and *above* CEF's own message pump. It therefore
exercises everything from the renderer inward, and bypasses exactly the OS→CEF path
where §4a would live. That asymmetry is what makes it decisive here — see §6.

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

---

## 6. Measurement (2026-08-31) — the in-page pipeline is PROVEN WORKING

Run against the live dev instance's floater over CDP, on a floating PowerShell
terminal. Findings in order:

1. **The DOM receives the event with `ctrl: true`**, and `closest('[data-blockid]')`
   resolves to the floater's real block id. So `ev.ctrlKey` is delivered correctly
   for an injected event.
2. **Propagation stops after body-capture** — it never reaches `window`'s bubble
   phase, so `AppZoomHandler` never runs. **This is by design, not the bug.** Each
   view registers its own capture-phase Ctrl+Wheel handler that calls
   `stopPropagation()` precisely so `AppZoomHandler` does not double-handle
   (`term.tsx:384`, `armory-view.tsx:78`, `editor-view.tsx:129`, `swarm-view.tsx:54`,
   `warden-view.tsx:55`). The **docked** window shows an identical trace, which is
   what establishes it as designed behaviour rather than a floater defect.
3. **The RPC is sent.** Captured off the wire by patching `WebSocket.prototype.send`:
   ```json
   {"command":"setmeta","data":{"oref":"block:3c936202-…","meta":{"term:zoom":2}}}
   ```
4. **The value round-trips.** Repeated injections accumulated 1.0 → 2.0 and clamped
   there, which is only possible if `termZoomAtom()` was reading back each write.
5. **The font is applied.** A screenshot showed the terminal text visibly enlarged,
   wrapping `PowerShell 7.6.5` across three lines.

**Conclusion: everything from the DOM event inward works in a floater.** §4b is dead.
Since CDP injection bypasses the OS→CEF path and succeeds where real hardware input
fails, **§4a is the surviving explanation by elimination**: a real Ctrl+Wheel from
the mouse does not arrive at the floater's renderer as a Ctrl-modified wheel event.

### 6.1 Two measurement errors made along the way

Both were caught, and both are the kind that produce confident wrong answers:

- **CSS `font-size` is meaningless here.** The wheel target is a `CANVAS` — xterm
  uses the canvas renderer, where `terminal.options.fontSize` never reaches CSS. An
  early probe read `14px` before and after and nearly concluded "zoom does nothing",
  when the screenshot shows it plainly working. Measure rows/cols or pixels, not
  computed style.
- **Capture-phase listeners cannot observe `defaultPrevented`** from a bubble-phase
  handler that has not run yet. A first probe reported `defaultPrevented: false` and
  it proved nothing.

### 6.2 What this means for the fix

§5 is **back on the table**, with its original premise restored: bypass the DOM by
intercepting `WM_MOUSEWHEEL` and reading the focus-independent `MK_CONTROL` bit.
All of §5's constraints still apply in full — hook the **CEF child hierarchy**, not
just `floating_pane_wndproc`, and **re-apply after every navigation**.

### 6.3 The one gap left

No control was obtained for **real** (non-injected) Ctrl+Wheel on a **docked**
terminal — the dev instance's main window had no terminal pane, and injected input
cannot answer it. The claim "works docked, not floating" still rests solely on the
reporter's observation.

The cheap discriminator, for a real mouse on a floating terminal: **does the terminal
buffer scroll when you Ctrl+Scroll?**

| Observation | Meaning |
|---|---|
| buffer scrolls | the wheel arrives but **the Ctrl modifier is stripped** — the handler's `if (!ev.ctrlKey) return` bails |
| nothing happens at all | **the wheel message itself** is consumed before the renderer |

These need different fixes, so this should be answered before code is written.
