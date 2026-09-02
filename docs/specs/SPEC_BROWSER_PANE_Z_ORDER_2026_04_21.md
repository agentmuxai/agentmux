# Spec: Browser pane Z-order fixes

**Date:** 2026-04-21
**Status:** Draft (analysis + implementation-ready)
**Scope:** Make main-window UI chrome (focus border, widget-bar popups, other overlays) appear above the browser pane's rendered content instead of behind it.

---

## Problem

Two visible bugs, same root cause:

1. **Focus border clipped.** The blue "pane is focused" border rendered by the block frame's CSS disappears over the edge of a browser pane. You see the border on three sides (where there's DOM margin) and nothing on the side flush with the pane content.
2. **Widget bar "... more" popup hidden.** Clicking the widget bar's overflow chevron opens a dropdown that extends over the workspace area. If a browser pane is under that area, the dropdown appears *behind* the browser's rendered page.

User expectation: everything adjacent to the browser content — focus border, dropdowns, context menus, tab previews, modals — is **above** the browser content.

---

## Root cause

Browser panes are **native CEF child HWNDs**, not CSS boxes. `pane/callbacks.rs:44-51` creates each pane with `SetWindowPos(HWND_TOP, ..., SWP_NOACTIVATE)` and the `browser_pane_resize` IPC positions it to fill `placeholderRef.getBoundingClientRect()` exactly.

**Windows compositing is OS-level, not browser-level.** When a child HWND exists within a parent, it's painted ON TOP of the parent's render surface — independent of CSS `z-index`, `position: fixed`, `transform: translateZ`, or any trick in the DOM. Main's Chrome_RenderWidgetHostHWND renders beneath the pane HWND because the pane sits higher in the sibling Z-order.

So: anything the main window draws through its Chromium render widget — focus border, popovers, modals, tooltips — is *physically below* the pane HWND and invisible wherever the pane overlaps it.

This is the classic "airspace" bug. The Chromium team documented it for `<iframe>` + `<embed>` twenty years ago; the same constraint applies to embedded CEF browsers. There's no pure-CSS fix.

## What the current code already does

- `pane::callbacks::on_after_created_pane` raises the pane to HWND_TOP so it sits *above main*, which is required for mouse-wheel events to route correctly (CEF widget hit-testing walks the HWND Z-order). See the comment at `browser_panes.rs:233`.
- `browser-view.tsx:34-40` syncs the pane HWND's position/size to a DOM placeholder's bounding rect on every render pass.

---

## Strategy (two orthogonal fixes)

### Fix A — focus border: shrink the pane rect by the border width

The focus border is a CSS border on the block frame (`.block-frame-default.is-focused`). It lives in the DOM, in main's render widget, which the pane HWND covers.

**The fix is to not cover it.** Shrink the pane HWND by the border thickness so the border strip is visible DOM where the pane isn't painted.

```
┌─────────────────── block frame ────────────────────┐
│  ┌──────── blue focus border (3-4 px CSS) ───────┐ │
│  │                                                │ │
│  │         pane HWND (shrunk by border)          │ │   ← pane now fits INSIDE the border
│  │                                                │ │
│  └────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────┘
```

Implementation: in `browser-view.tsx::paneRect()`, measure the border via `getComputedStyle` on the block frame and subtract it from the placeholder rect. Or simpler: make the placeholder itself be inset by the border width in the block layout (pure CSS change on `.block-content-browser` or the placeholder element), then `getBoundingClientRect()` returns the already-inset rect naturally.

Preferred: **CSS-only — inset the placeholder by the border width**. Zero IPC overhead, no JS math, the pane rect just follows the placeholder like it already does.

### Fix B — popups over the pane: hide/shrink the pane while a popup is open

CSS can't overlay a child HWND. The only Win32-level solutions are:

1. **Hide / shrink the pane temporarily** while the popup is open. Simple, reliable, has a small visible flicker on very fast show/hide. This is what VS Code and Electron apps do for similar cases.
2. **Move popups into their own top-level window** (layered HWND above the parent). Matches how native apps render tooltips. Bigger surface change, adds a second HWND to manage, interacts badly with theming/DPI.
3. **Switch panes to offscreen rendering (OSR)** where CEF paints into a bitmap that main blits into its own render surface. No HWND, no Z-order problem. Has knock-on costs: input plumbing, IME, hardware accel, accessibility all have to be re-wired through the host.
4. **Chromium's `<portal>` / `<fencedframe>`** — not applicable here; those are same-document primitives and we have two separate browsers.

**Option 1 is the pragmatic choice.** Scope it tight:

```
┌──────────────────────────────────────────────────┐
│  [widget bar ▾]  ← "...more" popup opens       │
│        │                                         │
│        ├── overflow-popup (DOM overlay)          │
│        │                                         │
│   ┌────┴───────────────────────┐                │
│   │ popup extends over the     │    pane HWND   │
│   │ workspace area here        │    hidden      │   ← visibility: hidden while popup visible
│   │                            │                │
│   └────────────────────────────┘                │
│                                                  │
└──────────────────────────────────────────────────┘
```

Implementation:
1. **Enumerate the overlay sources** that extend over pane territory:
   - Widget bar "… more" dropdown
   - Context menus (right-click in block frame, tab strip)
   - Focus border is Fix A, not B
   - Modals (already cover the full window; see below)
   - Agent settings overlay, identity panel overlay
2. **Central "any overlay open?" signal** in the frontend. A module-level signal (`overlayOpenAtom`) incremented by each overlay that needs to suppress panes, decremented on close. When it transitions `0 → 1`, call a new IPC `browser_panes_hide_all`. When it returns to 0, call `browser_panes_show_all`.
3. **Backend (`browser_panes.rs`):** `hide_all` iterates every live pane and calls `ShowWindow(hwnd, SW_HIDE)`; `show_all` does `SW_SHOWNOACTIVATE`. Cheap — one SetWindowPos-equivalent call per pane. Test count is small (1–3 panes typical).
4. **Full-screen modals already work** because the modal's backdrop uses `position: fixed; inset: 0` which is DOM, but since we'll be hiding the pane anyway when any overlay opens, modals get covered too without any extra code.

**Alternative (for future):** "Soft hide" — instead of `SW_HIDE`, SetWindowPos the pane to be 1×1 px in a corner. Keeps the browser's render process warm + layouted. Useful if `SW_HIDE` turns out to introduce its own flicker. Try `SW_HIDE` first.

---

## Files touched

### Fix A (focus border)

- `frontend/app/block/block.scss` (or whichever file owns `.block-frame-default` and `.block-content-browser`) — inset the placeholder element by the focus-border width. Might need a new class or a conditional rule for browser-view blocks only (terminals don't have this bug because the terminal is DOM-rendered, not native HWND).

### Fix B (hide panes under overlays)

- **Frontend**
  - New file or module-level store: `frontend/app/store/pane-overlays.ts` — `overlayOpenCount: signal<number>`, `incrementOverlay()`, `decrementOverlay()`, watch effect that fires the IPC on 0↔1 transitions.
  - Each overlay source calls `incrementOverlay()` on mount and `decrementOverlay()` on unmount:
    - `frontend/app/widget/widgetbar-overflow.tsx` (or wherever "… more" lives)
    - Context menu framework (once, central)
    - Agent settings overlay (`AgentCardSettingsPanel.tsx` or similar)
    - Identity panel overlay (`AgentIdentityPanel.tsx`)
- **Backend**
  - `agentmux-cef/src/browser_panes.rs`: add `fn hide_all(&self, state)` and `fn show_all(&self, state)` that walk live panes and `ShowWindow(SW_HIDE)` / `ShowWindow(SW_SHOWNOACTIVATE)` each outer HWND.
  - `agentmux-cef/src/ipc.rs`: add `browser_panes_hide_all` and `browser_panes_show_all` IPC commands that dispatch to the above (no args).

### Z-order semantics to preserve

- Still call `SetWindowPos(HWND_TOP)` on pane creation — mouse-wheel routing depends on it (`browser_panes.rs:226-231` comment).
- `SW_SHOWNOACTIVATE` on restore, NOT `SW_SHOW`, to avoid stealing keyboard focus.
- Hide/show must be idempotent — if the user opens the widget bar, then also opens a context menu, we shouldn't double-hide. That's what `overlayOpenCount` with transitions-on-boundaries handles.

---

## Non-goals

- **OSR mode for panes** (Option 3 in Strategy B). Bigger architectural change; worth revisiting if future requirements (e.g. blurred-glass overlays over pane content, tabs across panes that share a single view) demand it.
- **Real layered windows for tooltips / menus** (Option 2). Deferrable until we have an overlay source where the show/hide flicker is actually visible.
- **Per-pane hide** (only hide the pane the popup actually overlaps). Simpler to hide all; overlays are transient (sub-second typical).
- Transparent popup backgrounds that bleed through — purely cosmetic; hide-all implementation doesn't support this anyway since the pane disappears entirely.
- Linux/macOS equivalents. The `ShowWindow(SW_HIDE)` mechanism is Windows-specific; we'd gate the backend code on `#[cfg(target_os = "windows")]` and let mac/linux eat the behaviour (they're not the primary target yet).

---

## Edge cases

1. **Pane created while an overlay is already open** — new pane appears above the overlay, broken state. Mitigation: in `browser_pane_create`, check `overlay_count > 0` and immediately hide after creation. Or simpler: `browser_pane_create` IPC takes an extra `hidden: bool` arg defaulting to `false`; the frontend sets it to `true` when the overlay count is non-zero.
2. **Pane navigates / resizes while hidden** — SetWindowPos + navigate still work on a hidden HWND; state resumes correctly on show. No special handling.
3. **Pane HWND stuck hidden** — overlay count gets stuck at 1 because a component forgot to decrement on unmount. Safety net: mount-level cleanup in each overlay component, plus a devtools debug helper `window.__paneOverlayCount` for inspection.
4. **Focus restoration when pane re-shows** — don't auto-focus. User's focus target is whatever it was before the overlay opened; `SW_SHOWNOACTIVATE` preserves that.
5. **Pane browser on top of the whole window** (Fullscreen API) — out of scope; fullscreen is its own flow that re-parents the pane anyway.

---

## Test plan

### Fix A

- [ ] Focus a browser pane. Blue border is visible on all four sides.
- [ ] Resize the pane. Border stays correctly proportioned (not clipped on one edge).
- [ ] Unfocus / focus other panes repeatedly — no border ghosting on the previously-focused pane.
- [ ] Terminal / agent panes unaffected — their borders already work, this change shouldn't regress them.

### Fix B

- [ ] Open widget bar "… more" while a browser pane is present. Dropdown visible on top; pane content hidden behind it (or the pane is invisible, depending on implementation).
- [ ] Close the dropdown. Pane reappears in correct position.
- [ ] Right-click context menu over a browser pane — same thing.
- [ ] Agent settings overlay — panes hidden while overlay is up.
- [ ] Quick open/close: dropdown opens and closes within 100ms — no stuck-hidden pane.
- [ ] Two panes side-by-side: both hide/show together.
- [ ] Pane created while dropdown is open — edge case #1 — pane doesn't pop into view over the dropdown.

---

## Rollout

Two separate PRs is cleanest:

1. **PR A — focus border inset (CSS-only).** Tiny, low-risk, reviewable in 30 seconds. Merge standalone.
2. **PR B — overlay-aware pane hide.** Touches backend IPC + at least 3 frontend overlay components. Bigger review surface, more test coverage required.

If both fit comfortably in one diff (doubtful — overlay audit alone needs multiple frontend files), we can bundle; otherwise split. Ship A first.

---

## Future considerations

- **Accessibility.** Screen-reader users navigating while a pane is hidden: the hidden pane drops out of the AT tree for the duration; returning focus when the overlay closes should restore the expected focus target. Test with NVDA.
- **Hardware-accelerated overlays.** If we ever add a live preview pane (e.g. agent output streamed via WebGL into a popup), revisit — may justify Option 3 (OSR) at that point.
- **Cross-platform.** Mac / Linux have different window-system semantics. On macOS, NSWindow siblings can be layered more finely via `NSWindow.orderedIndex`; on Wayland it's a harder problem entirely.
