# SPEC — Browser pane: clicking inside it should dismiss open menus/popovers

**Date:** 2026-08-15
**Type:** Bug fix (design proposal — not yet implemented)
**Status:** Draft
**Scope:** Frontend (`frontend/app/window/action-widgets.tsx` and, if the
recommended fix is taken, every other "outside click" dismiss listener in the
app) + reuse of the existing `browser-pane-clicked` backend event
(`agentmux-cef/src/browser_pane/hwnd.rs` on Windows,
`agentmux-cef/src/ui_tasks/platform_macos.rs` on macOS). **No new backend
work required** — see Root cause.

## Problem

Reported by the user: open the widget bar's "More" dropdown, then click
inside a browser pane. The dropdown stays open. Clicking anywhere else in the
app (including elsewhere in the DOM, or another pane) closes it correctly —
only a click landing inside a browser pane fails to.

## Root cause

`action-widgets.tsx`'s "More" dropdown closes itself via a standard
outside-click listener:

```ts
// action-widgets.tsx:174-185
createEffect(() => {
    if (!moreOpen()) return;
    const handler = (e: MouseEvent) => {
        const t = e.target as Node;
        if (moreButtonRef?.contains(t) || moreDropdownRef?.contains(t)) return;
        const el = t instanceof Element ? t : (t as Node).parentElement;
        if (el?.closest(".popover-menu")) return;
        setMoreOpen(false);
    };
    document.addEventListener("mousedown", handler, true);
    onCleanup(() => document.removeEventListener("mousedown", handler, true));
});
```

This is a real DOM `mousedown` listener. Per
`docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`, a browser pane's content
is **not** in the DOM — it's a second, sibling `CefBrowserView` layered on top
of the main window via CEF's Views `AddOverlayView`. Once a page is loaded,
clicks on it are handled entirely by a separate native Chromium instance and
never become a DOM `mousedown`/`click` event in the host window. The listener
above simply never fires for those clicks — exactly the same root cause
already diagnosed and fixed (for pane *selection*, not menu dismissal) in
`docs/specs/SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md`.

**This is not unique to the widget "More" menu.** The same
`document.addEventListener("mousedown"/"pointerdown", ..., true)`
outside-click pattern is independently implemented in at least 18 other
places:

```
app/statusbar/TokenUsageIndicator.tsx   app/statusbar/HostPopover.tsx
app/statusbar/SystemStats.tsx           app/statusbar/StatusBar.tsx
app/statusbar/BackendStatus.tsx         app/tab/tab.tsx
app/tab/tab-reorder.ts                  app/element/flyoutmenu.tsx
app/element/popover-menu.tsx            app/components/context-menu.tsx
app/workspace/floating-pane-workspace.tsx (x2)
app/view/agent/components/AgentRuntimeDropup.tsx
app/window/action-widgets.tsx (x2 — More dropdown, item context-menu dismiss)
app/app.tsx (AppKeyHandlers)
```

Every one of these has the identical latent bug: a click landing on a browser
pane will not close it. The user only reported the widget menu because
that's what they happened to test, but a narrow fix scoped to
`action-widgets.tsx` alone would leave ~18 other reproductions of the same
bug in place.

## Existing infrastructure this can reuse

`browser-pane-clicked` already exists as a backend → frontend IPC event,
purpose-built for "a native click landed inside a browser pane, and the DOM
never saw it":

- **Windows**: `agentmux-cef/src/browser_pane/hwnd.rs` subclasses the pane's
  HWND tree and emits `browser-pane-clicked` (with `block_id`) directly from
  `WM_LBUTTONDOWN`.
- **macOS**: `agentmux-cef/src/ui_tasks/platform_macos.rs`'s
  `swizzled_nsapp_send_event` emits the same event on `leftMouseDown` for the
  pane's overlay window (added in
  `SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md`).
- **Linux**: not implemented yet (tracked as follow-up in the click-to-select
  spec). This fix inherits that same gap — see Scope/Caveats below.

Today the only consumer is `frontend/app/view/browser/browser-model.ts`,
which listens per-pane (filtering on its own `block_id`) and dispatches
`PaneClicked` to drive pane *selection* (the focus border). It does not do
anything else with the event.

## Proposed fix

Add **one** new, block-id-agnostic, always-mounted listener for
`browser-pane-clicked` that — on ANY browser pane being clicked, in any
window — synthesizes the same DOM signal every existing outside-click
listener already reacts to, instead of touching all ~18 call sites
individually.

```ts
// New: frontend/app/window/browser-pane-outside-click-bridge.ts (or inline
// in a top-level always-mounted component, e.g. alongside AppKeyHandlers in
// app.tsx)
onMount(() => {
    const unsubPromise = listenEvent<{ block_id: string }>("browser-pane-clicked", () => {
        // No block_id filtering — ANY pane click should be treated as an
        // "outside click" by every currently open dismissible menu/popover,
        // the same way clicking anywhere else in the app already is.
        //
        // Dispatched ON `document` (not `document.body`) so `e.target` is
        // `document` itself — never `.contains()`-matched by any menu/button
        // ref, and `instanceof Element` is false so existing handlers'
        // `.closest(".popover-menu")` fallback also safely no-ops instead of
        // throwing. Both `mousedown` and `pointerdown` are dispatched because
        // existing listeners are a mix of the two (see the file list above).
        for (const type of ["mousedown", "pointerdown"] as const) {
            document.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
        }
    });
    onCleanup(() => { void unsubPromise.then((unsub) => unsub()); });
});
```

Why this over a narrow `action-widgets.tsx`-only fix: it's roughly the same
amount of code, but fixes the bug class everywhere at once instead of
whack-a-moling 18 files (and however many more get added later that follow
the same copy-pasted pattern). It's purely additive — no existing listener
needs to change, since a synthetic outside-click event is indistinguishable
from a real one to code that only inspects `e.target`.

### Alternative considered (rejected)

Patch only `action-widgets.tsx`'s handler to also subscribe to
`browser-pane-clicked` and call `setMoreOpen(false)` directly. Simpler to
reason about in isolation, but leaves the other 18 sites (context menus,
status-bar popovers, tab rename, the item-level popover menu, floating-pane
drag-dismiss, etc.) broken. Rejected in favor of the generic fix unless a
reviewer specifically wants the smaller blast radius for a first pass.

## Scope / caveats

- **Linux gets no fix from this change**, same caveat as
  `SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md` — `browser-pane-clicked`
  is never emitted there today (no native click-detection mechanism exists
  yet for GTK/X11/Wayland). Menus will continue to stay open on a browser-pane
  click on Linux until that follow-up lands.
- Only affects *browser panes specifically* (native overlay content). Normal
  DOM panes are unaffected — they already worked correctly.
- Does not change `browser-model.ts`'s existing per-pane consumer of
  `browser-pane-clicked` (pane selection) — this is a second, independent
  subscriber to the same event, not a replacement.

## Verification plan (once implemented)

- Open the widget bar "More" dropdown, click inside a loaded browser pane →
  dropdown closes.
- Repeat for: a status-bar popover (Host/TokenUsage/SystemStats/Backend), a
  tab context menu, the pane header's own context menu on a *different* pane,
  a flyout/popover menu, the widget bar's per-item context submenu.
- Confirm normal in-pane browser interaction (scrolling, form input,
  clicking links) is unaffected — the synthetic event must not itself trigger
  anything unwanted inside the browser pane (it's dispatched on `document`,
  entirely outside the pane's own native overlay, so this should be a
  non-issue, but worth confirming).
- macOS + Windows only for now (see Scope).
