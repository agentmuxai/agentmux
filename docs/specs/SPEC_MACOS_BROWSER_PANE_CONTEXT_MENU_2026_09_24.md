# SPEC — macOS browser pane: show AgentMux's right-click menu above the page

**Date:** 2026-09-24
**Type:** Bug / platform parity
**Status:** implemented — PR #3736 (option A). Checked live
on a macOS dev build: the menu shows over the page, and Escape closes it and
restores the pane.
**Scope:** `frontend/util/cef-api.ts` (`showJsContextMenu`),
`frontend/app/platform/pane-overlay.ts` (`registerPaneOverlay`,
`isRegisteredPaneOverlay`), `frontend/app/util/menu-position.ts` (dev
guard). No Rust changes.
**Related:** `SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md` (#2599, the
cross-platform menu), `SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md` (#793, the
Windows-only auto clip), `SPEC_BROWSER_PANE_CLICK_DISMISSES_MENUS_2026_08_15.md` (#2597),
`agentmux-cef/src/ui_tasks/pane_hole_mask.rs` (#2098 / #2130, the macOS hole punch).

## Problem

On Windows, right-clicking inside a browser pane's page shows AgentMux's own
menu at the cursor, drawn over the live page: Back, Forward, Reload, Cut, Copy,
Paste, Copy Link Address, Print, View Page Source and Inspect Element, then the
shared pane items (Split, Replace With…, Magnify, Close Pane).

On macOS the same right-click builds the same menu, but you can't see or use
it. The menu draws in the main window underneath the pane's native window.

## Observed on macOS (dev build, main @ 1d02df120, 2026-09-24)

I sent a real OS right-click (CGEvent) into an `example.com` browser pane:

- CEF's `run_context_menu` fired and suppressed Chromium's native menu.
- `browser-pane-context-menu` reached the frontend.
- `showJsContextMenu` built the full menu at the cursor. The menu rect was
  `950,269 125x423`, and all the expected items were present.
- The app's own guard logged the failure:
  ```
  [menu-guard] "context-menu" rendered outside the paintable area:
  behind native pane (pane 805,108 391x665) — menu rect 950,269 125x423
  ```
- No `browser_panes_set_overlay_clip` IPC was sent. The pane stayed fully
  opaque over the menu.

So everything up to drawing the menu is already cross-platform and working.
Only the "show the menu above the pane" step is missing on macOS.

## How it works on Windows (reference)

| Step | Where | Platform-specific? |
|---|---|---|
| 1. Right-click reaches the pane's Chromium view | Native. Windows: child window. macOS: `NativeWidgetMacNSWindow` overlay; right-mouse-down (ev 3) on the overlay falls through `swizzled_nsapp_send_event` (`ui_tasks/platform_macos.rs:229`) untouched | per-OS, already works on both |
| 2. `run_context_menu` captures x/y, link, selection, editable, back/forward, target frame; cancels Chromium's menu; emits `browser-pane-context-menu` to the pane's owning window | `agentmux-cef/src/client/context_menu.rs:38-123`, registered for every browser pane in `client/handlers.rs:46-55`, no `cfg` | shared |
| 3. Block frame maps pane x/y to window CSS px using `.browser-placeholder`'s rect and calls `onBodyContextMenu` | `frontend/app/block/blockframe.tsx:1233-1285` | shared |
| 4. Items: `BrowserModel.getBodyContextMenuItems` + `buildPaneContextMenu` | `browser-model.ts:650-739`, `block/pane-actions.ts:149-222` | shared |
| 5. Render a DOM menu with a full-window backdrop; `.menu` and submenus are tagged `data-pane-overlay` | `showJsContextMenu`, `frontend/util/cef-api.ts:151-395` | shared |
| 6. **Punch the menu's rect out of the pane so it shows through** | `pane-overlay-auto.ts` sees `[data-pane-overlay]` → `pane-overlay.ts sendClip()` → `browser_panes_set_overlay_clip` → `SetWindowRgn` (`browser_panes/clip.rs:58-351`) | **Windows only: the auto service returns early off Windows** |
| 7. Actions run back on the pane via `browser_pane_go_back/forward/reload/print/view_source/inspect_element/cut/copy/paste` | `agentmux-cef/src/ipc.rs`, `browser_panes/navigation.rs` | shared |

## Root cause

Step 6. `showJsContextMenu` doesn't register its own rect. It only sets
`data-pane-overlay` and relies on the auto-discovery service to notice. That
service is switched off on every platform except Windows:

```ts
// frontend/app/platform/pane-overlay-auto.ts — startPaneOverlayAutoService()
if (typeof navigator !== "undefined" && !navigator.userAgent.includes("Windows")) {
    return;
}
```

The gate was added in #793 (2026-05-11). At that time macOS pane overlays
really were a no-op (`SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md:282`). That is
no longer true. Since #2098 and #2130, macOS handles
`browser_panes_set_overlay_clip` with a real hole punch:

- a `CAShapeLayer` even-odd mask on the overlay window's content layer
  (`ui_tasks/pane_hole_mask.rs`, via the macOS branch of
  `SetPaneOverlayClipViewsTask`, `browser_panes/clip.rs:580-643`);
- `ignoresMouseEvents = YES` on the whole overlay window while any hole is
  active, so clicks and hovers reach the DOM menu underneath.

Solid-rendered menus (`FlyoutMenu`, `PopoverMenu`, and the status-bar popovers)
call `usePaneOverlay()` directly, so they already work over macOS panes.
`showJsContextMenu` is imperative, not a Solid component, so it can't use the
hook and never registers anything. The comment in `showJsContextMenu` saying
"the `data-pane-overlay` clip reveals it through the native pane" is only true
on Windows.

## Fix

### 1. Imperative overlay registration (core fix)

Add a non-hook API to `frontend/app/platform/pane-overlay.ts`, next to
`usePaneOverlay`, that shares the same `overlayRects` map and `sendClip()`:

```ts
/** Register `el`'s current rect as a pane overlay. Returns a handle to
 *  re-measure after moves and to release. Safe on every platform: sendClip()
 *  already routes to the host, which is a no-op where there are no panes. */
export function registerPaneOverlay(el: HTMLElement): { update(): void; release(): void };
```

Use it in `showJsContextMenu`:

- Register `menuEl` once `applyMenuPosition` has placed it. It's held at
  `visibility:hidden` until then, so the rect registers once, at its final
  position.
- Register each submenu when it opens, after it is positioned, and release it
  when it closes (`peer.close()` / the submenu hover controller).
- Release everything on **every** close path. Today they are: a backdrop
  mousedown (`cef-api.ts:165`), an item click (`:353`), and a new menu
  replacing an old one (`:157`, `getElementById(...)?.remove()`). Route all
  three through one `closeMenu()` that releases the handles and then removes
  the backdrop. A leaked registration leaves a transparent, click-through hole
  in the pane. That's the worst failure mode here, so this part is required.
- Don't register the full-window backdrop (`#cef-context-menu-overlay`). It is
  not tagged today, and must stay that way, or the whole pane gets masked.

This also fixes the same gap on Linux, where the host's fallback is to hide the
whole pane while the menu is open. It doesn't change Windows: the auto service
there already registers these elements, and both paths union into one clip.
Confirm there's no double-apply flicker on Windows (§6).

### 2. Keep the right-click selecting nothing new

Out of scope. On both platforms a right-click in the page doesn't select the
pane (only left-mouse-down emits `browser-pane-clicked`). That's existing
Windows behaviour and parity is the goal. It can be a follow-up.

### 3. Dismissal (small, same PR)

- **Click into the pane while the menu is open.** On macOS this already
  works: while the hole is active, the whole overlay window ignores the mouse,
  so the click lands on the full-window backdrop and closes the menu. On
  Windows it's a probable gap. A click on the pane outside the hole goes to
  the pane, and `browser-pane-outside-click-bridge.ts` sends a synthetic
  mousedown to `document.body`, which never matches the backdrop's
  `e.target === overlay` check. Have the bridge (or a `browser-pane-clicked`
  listener) call the same `closeMenu()`.
- **Escape.** `showJsContextMenu` has no keyboard dismissal on any platform.
  Add an Escape keydown listener, attached on open and removed in
  `closeMenu()`. On macOS, once the page can hold the keyboard
  (`SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24.md`), the host hands key
  status back to the app window whenever a hole is punched, so Escape
  reaches the DOM even if the page was being typed into.

### 4. Dev guard (as implemented)

`assertMenuInPaintableArea` (`menu-position.ts`, dev builds only) reported
any menu overlapping a pane as "behind native pane". It now skips that check
for elements registered to punch a hole (`isRegisteredPaneOverlay`: explicit
registration, or the Windows auto service). It still catches the bug this
spec fixes, a tagged but unregistered menu.

## 4. Options considered

**A. Imperative registration in `showJsContextMenu` (recommended).** Small,
targeted, and uses the exact mechanism the Solid menus already use on macOS.
The risk is limited to one menu implementation.

**B. Turn `startPaneOverlayAutoService` on for macOS.** This fixes every
`data-pane-overlay` element at once, not just this menu. It is broader than
this bug, and on macOS it has a real cost: any tagged tooltip or popover that
overlaps a pane makes the **entire** pane ignore the mouse while it's shown
(the `ignoresMouseEvents` trade-off in `pane_hole_mask.rs`). Every tagged
element would need auditing first. Leave it for a separate spec. If B lands
later, option A's calls stay harmless, because each registration is keyed by
element.

**C. Native `NSMenu` for panes on macOS.** Rejected. It would fork menu
contents and styling per platform, and the host command `show_context_menu`
is already a deliberate no-op in favour of the DOM menu.

## 5. Coordinates and scaling (verify during implementation)

- In the observed run the click was at window-local CSS `(949,260)` and the
  menu landed at `(950,269)`. X matched. Y was 9px lower, which looks like the
  positioning framework's cursor offset. Confirm the menu's top-left sits at
  the cursor, as it does for a right-click on a non-browser pane.
- `run_context_menu`'s `xcoord/ycoord` are view DIPs. `blockframe.tsx` adds
  them to the placeholder rect in CSS px unscaled. That's correct when the
  main window zoom is 1 and the page zoom is 100%. Check both with a zoomed
  main window (Armory zoom) and a zoomed pane (per-pane browser zoom,
  `REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md`).
- `sendClip()` multiplies by `devicePixelRatio`, and the macOS path compares
  against `browser_pane_physical_rects`. Confirm on a Retina display
  (DPR 2), and on a non-Retina external monitor if available, that the hole
  lines up with the menu edges without a 1px seam.

## 6. Test plan (macOS dev build)

1. Right-click on plain page text: the menu is visible over the page at the
   cursor, and the pane around it keeps painting live (e.g. a video keeps
   playing).
2. Hovering menu rows highlights them. Clicking Reload reloads the pane,
   Back/Forward navigate, and View Page Source loads `view-source:`.
3. Right-click a link, then Copy Link Address: the clipboard gets the URL.
4. Right-click in a text field (e.g. Google's search box), then Paste and Cut:
   they act on that field (the stored target frame).
5. Open "Replace With…": the submenu is visible even where it overlaps the
   pane, and hovering it works.
6. Menu partly outside the pane (right-click near the pane edge): both parts
   are visible, and only the overlapping part is punched out.
7. Dismiss by clicking the backdrop outside the pane, by clicking inside the
   pane, by pressing Escape, and by picking an item. In every case the pane
   afterwards has no hole and accepts clicks (check that
   `ignoresMouseEvents` is back to `NO`: a left-click selects the pane and
   emits `browser-pane-clicked`).
8. Right-click again while a menu is open (the replacement path): no leaked
   hole from the first menu.
9. Floating and torn-off pane windows: the menu shows in the pane's own
   window (the event already targets `browser_pane_window_label`), and the
   clip is sent with that window's label.
10. Two panes side by side, menu spanning both: both get holes, and both are
    restored on close.
11. Regression, Windows: no change in behaviour and no extra flicker from the
    double registration (auto service plus explicit).
12. Linux (if available): the pane hides while the menu is open instead of
    covering it.

Instrumentation already exists for most of this: the `[menu-guard]` warning
(`frontend/app/util/menu-position.ts`) should no longer fire for
`"context-menu"`, and the host logs `[pane-airspace] views: applied hole mask`
with `hole_count` on every clip change.

## Out of scope

- Enabling the auto overlay service on macOS (option B).
- Right-click selecting the pane.
- Menu contents: the duplicate "Copy" (always disabled for browser panes)
  and the duplicate main-window "Inspect Element" from `buildPaneContextMenu`
  should be dropped for browser panes, but that is a cross-platform content
  change.
- Keyboard input into macOS browser panes (panes can't become the key window,
  `can_activate=0`). That's a separate bug, found in the same investigation.
