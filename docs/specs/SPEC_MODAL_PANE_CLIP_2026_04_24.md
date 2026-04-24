# Spec: Modal-v2 ↔ Native Pane Airspace Clipping

**Date:** 2026-04-24
**Status:** Ready to implement
**Owner:** AgentA
**Touches:** `frontend/app/platform/pane-overlay.ts`, `frontend/app/element/modal-v2.tsx`

---

## 1. Problem

Modal-v2 renders as a Solid.js `<Portal>` at `z-index: var(--z-modal)`
(`frontend/app/element/modal-v2.scss:31`). Native browser panes are
CEF `BrowserView` child HWNDs composited by Windows **above** the main
HTML renderer's surface — CSS z-index has no authority over Win32
z-order. Result: when a modal opens while a browser pane is visible,
the pane's HWND paints on top of the modal backdrop and panel,
leaving the modal partially (or fully) invisible.

This is the "airspace" problem explicitly called out in
`docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md:278-280`:

> Z-order: The browser overlay always renders on top of the main UI.
> If a modal or context menu opens, it may render behind the browser
> pane. **Fix: hide the browser view when modals are open.**

## 2. Existing infrastructure

The fix is *already in the repo* — just not wired to modal-v2. The
mechanism used by the `MoreDropdown` (action-widgets) subtracts the
overlay element's screen rect from each pane HWND's visible region
via Win32 `SetWindowRgn(... RGN_DIFF ...)`:

| Layer | File:line | What it does |
|---|---|---|
| Frontend hook | `frontend/app/platform/pane-overlay.ts:59-72` | `usePaneOverlay(getEl)` reads `getBoundingClientRect`, stores in a module-level `Map`, sends the union to IPC |
| IPC | `agentmux-cef/src/ipc.rs` → `browser_panes_set_overlay_clip` | Forwards rects to the lifecycle manager |
| Backend | `agentmux-cef/src/browser_panes.rs:343-423` | For every live pane HWND: builds a region = pane-rect minus every intersecting overlay rect; `SetWindowRgn`. Empty list → `NULL` region → full visibility restored |
| Consumer | `frontend/app/window/action-widgets.tsx:144-214` (`MoreDropdown`) | Calls `usePaneOverlay(() => overlayEl)` on the dropdown's outer div |

The registry is shared and union-based — multiple overlays stack
cleanly (`sendClip()` sends the Array of all current rects every time).

## 3. Why hide-the-pane is not the answer

An alternative is "hide the browser view entirely while any modal
is open" — simpler in principle, but:

- **Jarring UX.** The pane's entire viewport blanks as soon as you
  click *any* button that opens a confirm dialog. Scroll position,
  inline playback, loading state all interrupt.
- **Flicker on close.** Unhide = brief reload / repaint.
- **Doesn't compose with partial-viewport overlays.** If a future
  modal variant renders in a corner rather than full-screen, the
  pane should stay visible in the uncovered area.

Clipping gives us the right UX — the pane is visually subtracted
only where the modal backdrop actually paints. Matches what
MoreDropdown already does. No new primitive.

## 4. What needs to change

Two small deltas:

### 4.1 Enhance `usePaneOverlay` to track window-resize

Today the hook reads the element's rect **once** at mount. That's
fine for the dropdown (static, position:fixed, small rect) but
insufficient for modal-root (position:fixed; inset:0 — the rect is
the viewport and therefore changes on any window resize).

Change: add a `window.resize` listener that re-reads the rect and
re-`sendClip()`s. Cleaned up in `onCleanup`.

```ts
export function usePaneOverlay(getEl: Accessor<HTMLElement | null | undefined>): void {
    const id = nextOverlayId++;
    const update = () => {
        const el = getEl();
        if (!el) return;
        overlayRects.set(id, rectFromElement(el));
        sendClip();
    };
    onMount(() => {
        update();
        window.addEventListener("resize", update);
    });
    onCleanup(() => {
        window.removeEventListener("resize", update);
        if (overlayRects.delete(id)) {
            sendClip();
        }
    });
}
```

Backward-compatible: MoreDropdown already works; now it also
self-heals on resize (previously its rect would go stale — minor
pre-existing bug, fixed for free).

### 4.2 Wire modal-v2's backdrop rect into the hook

Modal-v2's JSX already has a stable `.modal-root` div that covers
the viewport (`position: fixed; inset: 0`). That div is the right
overlay element — registering its rect subtracts the entire
viewport from each pane, which is exactly what we want: the pane
goes invisible for the duration of the modal, and the HTML backdrop
+ panel render in the freed airspace.

Integration needs three things:

1. Capture a ref on `.modal-root`.
2. Call `usePaneOverlay(() => rootRef)` **inside the `<Show>` tree**
   so the hook's `onMount` / `onCleanup` fire on every open/close,
   not once per component instance.
3. Because Solid's hooks must be called in a reactive owner created
   by the `<Show>`, introduce a tiny presentational helper
   component that is mounted inside the Show:

```tsx
const ModalPaneOverlayClip: Component<{ getEl: Accessor<HTMLElement | undefined> }> = (p) => {
    usePaneOverlay(p.getEl);
    return null;
};
```

In the Modal JSX:

```tsx
<Show when={mounted()}>
    <Portal mount={resolveMountDocument().body}>
        <ModalTitleIdContext.Provider value={defaultTitleId}>
            <div class="modal-root" ref={rootRef} …>
                <ModalPaneOverlayClip getEl={() => rootRef} />
                …existing contents…
            </div>
        </ModalTitleIdContext.Provider>
    </Portal>
</Show>
```

Stacked modals each mount their own clip — they register identical
viewport rects, the backend unions them (identical rect is a no-op
in region math), and unmount peels them off individually. This
works for out-of-order closes already handled by the modal
document-lock refcount.

## 5. Why the modal rect, not the panel rect

Option A (what this spec picks): register the **full** `.modal-root`
rect (viewport). The backdrop DOM scrim paints uniformly across
the screen including over where the pane was.

Option B: register only the `.modal-panel` rect. Backdrop would
show through the center but the pane would render outside the
panel — the scrim wouldn't darken the pane area, leaving the
semi-transparent backdrop visually invisible over the pane. The
modal would look like a dialog floating over a fully-awake browser
pane, which is exactly the UX the scrim is there to prevent.

A is correct.

## 6. Implementation steps

1. **Enhance hook.** Edit `frontend/app/platform/pane-overlay.ts`:
   add the window-resize listener per §4.1. Update the JSDoc
   comment to remove the "not yet needed" caveat about
   dynamically-sized overlays.

2. **Add `ModalPaneOverlayClip` helper** inside `modal-v2.tsx` (not
   exported — internal).

3. **Capture modal-root ref** and wire the helper inside the `<Show>`.

4. **Build + lint + type-check.**
   - `task build:frontend` — succeeds
   - `npm run lint:scss` (no change expected, but should be green)
   - `npx tsc --noEmit` — clean

5. **Manual smoke.** With `task dev`:
   - Open a browser pane (default URL).
   - Trigger any modal (About, ConfirmModal delete, launch modal).
   - Modal backdrop + panel should paint over the pane. No pane
     content visible through the panel or backdrop.
   - Close modal. Pane snaps back to full visibility.
   - Open a second modal on top of the first (e.g. nested confirm).
     Still invisible pane.
   - Close inner modal (outer stays open). Still invisible pane.
   - Close outer. Pane returns.

6. **Resize test.** Resize the AgentMux window while a modal is
   open. Backdrop should follow the new viewport and still cover
   the pane fully.

7. **Second-window smoke (if reachable).** Open an additional CEF
   window (if the app supports it) with its own browser pane and
   modal; verify the clip applies to *its* pane HWNDs only
   (already true — the backend iterates `lifecycle.live_labels()`
   which is window-scoped).

## 7. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Resize listener on `window` fires frequently during a drag-resize | The update is O(pane count) + one IPC; MoreDropdown's coexistence shows the cost is fine. Can add a `requestAnimationFrame` coalesce later if hot. |
| Modal rect is registered on mount **before** the Portal's paint. If the rect is read before the rootRef is attached, it registers `null` and skips. | Portal mounts synchronously before Solid's `onMount` fires for children inside it, so the ref is already set by the time `usePaneOverlay`'s `onMount` runs. Matches the same-frame sequence MoreDropdown already depends on. |
| A future modal variant renders *without* covering the viewport | The helper registers whatever element's ref it's given. Callers pick. Default continues to be the full-viewport `.modal-root`. |
| Two modals with identical rects — backend region math | `RGN_DIFF` of identical rects is idempotent; a second subtraction is a no-op. Safe. |
| Multi-window: modals in window B wrongly clip panes in window A | `set_pane_overlay_clip` iterates every live pane label globally — today modals in B *would* clip A's panes. In practice all modals are full-viewport and today's users run one window. Flagged as a follow-up in §9 (not a blocker for this PR). |

## 8. Non-goals

- No change to the backend IPC / Rust clipping code — it already
  handles multi-rect unions correctly.
- No migration of non-modal DOM overlays (tooltips, popovers). Each
  such overlay can opt in by calling `usePaneOverlay` itself.
- No macOS / Linux work. Pane HWNDs are Windows-only; the hook is
  a no-op elsewhere because the backend is `#[cfg(target_os =
  "windows")]`.
- No "hide the pane" fallback. Clip is the chosen path.

## 9. Follow-ups (not in this PR)

1. **Per-window scoping.** Today clip lists are global; a multi-
   window setup would cross-clip. Scope overlay registration to
   the window that owns the Portal root. Wait until multi-window
   UX is genuinely shipping before investing.
2. **rAF coalesce.** If resize perf ever regresses, wrap `sendClip`
   in a `requestAnimationFrame` coalescer.
3. **Context menu.** Context menus (right-click) are rendered via
   native CEF menus and not affected — but if we ever switch to
   DOM context menus, same `usePaneOverlay` call applies.

## 10. Validation

- ✅ `tsc --noEmit` passes
- ✅ `task build:frontend` succeeds
- ✅ Manual smoke (§6.5 + §6.6) succeeds
- ✅ No regression of `MoreDropdown` (same hook, extra resize
  listener is additive)

## 11. Cross-references

- `docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md` — origin of
  the airspace problem callout.
- `docs/specs/SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md` — modal-v2's
  own design (this is a targeted fix, not a new primitive).
- `frontend/app/platform/pane-overlay.ts` — the hook.
- `agentmux-cef/src/browser_panes.rs:343` — `set_pane_overlay_clip`.
