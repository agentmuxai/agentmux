# SPEC — active-tab color line: stop at the tab strip's right edge, not the viewport edge

**Date:** 2026-07-13
**Author:** Agent2
**Status:** Superseded — the whole `active-tab-color-line` feature this
spec narrows was removed outright on 2026-08-24 (repo-owner-confirmed
request: "we want to eliminate those lines that span under the tab bar").
`frontend/app/tab/tab-color-line.ts` deleted; `tabbar.tsx`'s
`<Portal>`-rendered `.active-tab-color-line` div removed. The per-tab
`tab:color` feature itself (the whole tab's own background tint —
`.tab-colored` in `tab.scss`) is unaffected; only this full-strip
underline is gone. Kept here as historical record of the geometry
decision, not current behavior.
**Scope:** `frontend/app/tab/tabbar.tsx` (the `measureLine`/`active-tab-color-line` mechanism only — no other tab-bar behavior).
**Related (must-read first):**
`docs/specs/SPEC_TAB_CONTENT_FOLDER_SURFACE_2026_06_03.md` §10.6 (the plain hairline-boundary mechanic this feature extends with color),
commit `99738cd1` / PR #1979 (feature introduction: "active tab's custom color traces the tab strip's boundary line"),
commit `d1e990d9` (the right-edge-to-viewport decision this spec reverses — see §2),
commit `dbe5ce48` / PR #1988 (the companion left-edge fix — already correct, unaffected by this spec).

---

## 1. Summary

The active tab's custom `tab:color` paints a 3px line under the tab strip (`.active-tab-color-line`, `tabbar.tsx`). Its **left** edge correctly starts at the selected tab's own left edge (fixed in PR #1988). Its **right** edge currently runs `window.innerWidth - left` — i.e. all the way to the window's right edge, through the empty tab-bar space, the header widgets (`.system-status`/`ActionWidgets`), and (on Windows/Linux) under the window control buttons.

The user wants the right edge to stop where the actual tabs stop — at the end of the tab strip's content, not the end of the window. This spec proposes measuring the right edge from `.tab-bar-fill` (the existing flex-filler element that already marks exactly that boundary) instead of `window.innerWidth`.

## 2. Where we are today — and why this is a deliberate reversal, not a plain bug

This is worth stating plainly: the current full-viewport-width behavior was not an oversight. It was a specific design decision made via a live A/B preview with the user in an earlier session. Commit `d1e990d9`'s message:

> "Live preview against the previous stopping-before-window-controls version, done with the user: this one — running under the window control buttons too, all the way to the right edge — was picked. Simplifies the boundary calc since there's no longer a platform-conditional stop point to look up."

So the two options already tried were "stop before the window control buttons" (i.e. stop at `.system-status`'s right edge / before `.window-action-buttons`) vs. "run all the way to the viewport edge" — and the viewport-edge version won that comparison. This spec's proposal — stop at the **tab strip's own content edge** (i.e. right after the last tab, well before `.system-status` even begins) — is a **third, narrower option that was not part of that earlier comparison**. It's not re-litigating the same choice; it's a different, more conservative stopping point than either of the two previously compared. Noting the history so the reversal is understood, not silently overwritten.

**Current implementation** (`tabbar.tsx`, `measureLine`, ~line 906-921):
```ts
const measureLine = (): boolean => {
    if (!tabBarRef) return false;
    const activeTabEl = tabWrapperRefs.get(activeTabId());
    if (!activeTabEl) return false;
    const left = activeTabEl.getBoundingClientRect().left;
    setLineLeft(left);
    setLineBottom(window.innerHeight - tabBarRef.getBoundingClientRect().bottom);
    // Right edge runs all the way to the viewport's right edge — ...
    setLineWidth(window.innerWidth - left);
    return true;
};
```
Left edge: the selected tab's own wrapper (`tabWrapperRefs.get(activeTabId())`), correct and unaffected by this spec. Right edge: `window.innerWidth`, the thing to change.

**The tab strip's actual right boundary already exists in the DOM**, and is exactly the point the user wants the line to stop at: `.tab-bar-fill` (`tabbar.tsx` ~line 1035, `tabbar.scss` ~line 181) — a flex-filler `<div>` rendered as the last child of `.tab-bar-scroll`, immediately after the `<For>` over tabs with no separator in between (`tabbar.tsx` ~line 1000-1035). Its whole purpose is to absorb the flex space to the right of the last tab (`flex: 1 1 auto`) so that empty area is still draggable — its own left edge is therefore, by construction, flush with the last tab's right edge, regardless of tab count, tab widths (content-aware sizing per `SPEC_TAB_CONTENT_AWARE_SIZING_2026-06-14.md` means tab widths aren't fixed), or scroll position.

## 3. Proposed design

Measure the line's right edge from `.tab-bar-fill`'s left edge instead of `window.innerWidth`:

```ts
const right = tabBarFillRef?.getBoundingClientRect().left ?? window.innerWidth - left; // fallback only if the ref genuinely isn't available
setLineWidth(right - left);
```

**Ref, not a query selector** — `tabWrapperRefs`/`tabBarScrollRef`/`tabBarRef` are already Solid refs threaded through this component; add `tabBarFillRef` the same way (a `ref={tabBarFillRef!}` on the existing `.tab-bar-fill` div) rather than a `document.querySelector(".tab-bar-fill")` call, for consistency with the rest of this file and to avoid a global DOM query on every measurement.

**Re-measurement triggers** — everything that already re-measures today (`ResizeObserver` on `tabBarRef`/`tabBarScrollRef`, `window resize`, scroll, the `activeTabId()`/`tabIds()` effect) remains correct and sufficient: `.tab-bar-fill`'s position moves whenever tab count or tab widths change, both of which are already covered by the `tabIds()` dependency (add/remove/reorder) and the `ResizeObserver` on `tabBarScrollRef` (content-aware width changes on existing tabs, e.g. a tab's title changing length). No new observer needed — just add `tabBarFillRef` to the existing `ro.observe(...)` calls in `onMount` for completeness (its own box doesn't need observing since it has no intrinsic size to change, but observing it costs nothing and guards against a future layout change to `.tab-bar-fill` itself going unnoticed).

**Edge case — a single tab, or tabs that exactly fill the strip:** if the tabs already fill (or overflow) the visible tab-bar width, `.tab-bar-fill`'s box has zero width and sits flush against the last tab — the line's right edge simply lands there, which is correct (there's no "extra" space to visually not extend into).

**Edge case — RTL / very narrow windows:** not applicable; the tab bar is LTR-only today (no `dir="rtl"` handling anywhere in this component), consistent with the rest of the app.

## 4. What NOT to change

- The **left edge** (`tabWrapperRefs.get(activeTabId())`) — already correct per PR #1988, untouched by this spec.
- The **plain hairline boundary** (`.tab:not(.active)`'s `border-bottom`, continued across `.tab-bar-fill` per `SPEC_TAB_CONTENT_FOLDER_SURFACE_2026_06_03.md` §10.6) — a separate, always-present, uncolored line this spec does not touch. Only the colored `active-tab-color-line` overlay changes.
- The **Portal-to-`document.body`** rendering strategy (needed to escape `.tab-bar`'s `overflow: hidden`) — still needed even with a shorter line, since the line still needs to render outside `.tab-bar-scroll`'s own `overflow-x: auto` clipping to avoid moving with horizontal scroll of tabs positioned near the strip's right edge. Unaffected either way.

## 5. Tests

- Unit/DOM test (if `tabbar.tsx` gains test coverage for this, or extending existing coverage): with N tabs of varying widths, `lineWidth` + `lineLeft` should sum to `.tab-bar-fill`'s `getBoundingClientRect().left`, not `window.innerWidth`.
- Manual: resize the window wider — the line's right edge should stay pinned to the last tab, not stretch to the new window edge. Add/remove tabs — the line's right edge should track the new last-tab boundary. Select a tab while few tabs are open (line is short) vs. many tabs open (line may span most of the strip) — both should stop exactly at the last tab, never past it.

## 6. Why this is worth doing

The colored line's whole purpose (per #1979's own commit message) is to give "a persistent, colored indication of which tab you're on beyond the tab itself" — i.e., a boundary-line variant of the tab-strip metaphor, not a whole-window status bar. Running it through the header widgets and window controls to the viewport edge dilutes that reading: at a glance it looks like a window-wide accent bar unrelated to tab count, rather than an extension of the tab strip specifically. Stopping it at the tabs' own right edge — the boundary `.tab-bar-fill` already marks for exactly this purpose (delimiting "where the tabs end") — makes the line legible as what it actually is.
