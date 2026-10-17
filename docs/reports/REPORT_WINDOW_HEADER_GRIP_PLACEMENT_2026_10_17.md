# Report: Where the window header's grip sits, and the gap before the window controls

**Date:** 2026-10-17
**Author:** Maricon@charlie
**Status:** implemented — one placement for every platform: `WindowGrip` inside `.system-status` right after the widget bar, so it is before the window controls on Windows and Linux, and before the hamburger (which follows `.system-status`) on macOS.
**Requested by:** the operator: *"the window header grip currently sits to the left of the widgets. Instead, we want to bring it to the right of the widgets between the widgets and the window controls. Aside from that, I notice an unnatural gap between the widgets and the window controls. This applies only to windows and linux. On macos, the grip is to move between the widgets and the hamburger on the right side."*
**Related:** `docs/specs/SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md` §5.1 and §5.8 (where the grip is today, and why), `docs/specs/SPEC_LINUX_WINDOW_CONTROLS_GNOME_STYLE_2026_10_16.md` (the Linux controls).

## 1. Summary

- **Move the window grip** out of the tab bar.
  - Windows and Linux: between the widgets and the window controls, `[tabs …] [widgets] [grip] [– ▢ ×]`.
  - macOS: between the widgets and the hamburger, `[traffic lights] [tabs …] [widgets] [grip] [☰]`.
- **The gap.** It comes from three paddings that add up, not from a single bug: the widget bar's "More" button pads its right side by 8 px, `.system-status` adds a 6 px gap, and a Windows 11 caption button centres a 10 px glyph in a 46 px tile, which leaves 18 px of empty tile. That is about 32 px from the More chevron to the Minimize glyph, while widget icons sit about 17 px apart. Moving the grip into that space fixes most of it. The other part is to make the gaps on either side of the grip match the widgets' 4 px gap, so the spacing reads as one rhythm.
- **Pane headers:** unchanged. Their grip already sits just before the pane controls (§5.8 of the grip spec).
- It is a small, frontend-only change.

## 2. What the code does today (main at `7005dae3a`)

### 2.1 The window header

`frontend/app/window/window-header.tsx` lays the header out as:

```
[WindowControlsLeft] [WindowDrag] [TabBar: tabs … fill | grip] [SystemStatus: widgets | controls] [macOS: ☰]
```

- **The grip** is `.tab-bar-grip` in `frontend/app/tab/tabbar.tsx`: a fixed 32 px drag square (`data-drag-region="true"`) after the scrolling tab strip and before `SystemStatus`. That puts it to the left of the widgets. It is drawn by `GripDots` (`frontend/app/element/GripDots.tsx`) and takes no events itself.
- **Why it is there:** the owner's choice of 2026-10-13: *"in the window tabs, it gets mounted to the left of the left most widget … so that the scroll and fade only apply to the tabs, not the grips"* (grip spec §5.8). The new placement keeps that property, because the grip stays outside the scrolling strip.
- **`SystemStatus`** (`frontend/app/window/system-status.tsx`) holds the widgets (`ActionWidgets`) and, on Windows and Linux, the window controls (`WindowControlsRight`). Its CSS has `gap: 6px` and `margin-right: 6px`.
- **macOS:** the window controls are the traffic lights on the left (`WindowControlsLeft`), and `HamburgerMenu position="right"` is rendered after `SystemStatus`.

### 2.2 Measured on Linux

Measured on a build of current main (0.59.30, the GNOME-style controls from #5039), 1200 px wide window, over the debug port:

| Element | Left | Right | Width |
|---|---|---|---|
| `.tab-bar-grip` | 631 | 663 | 32 |
| `.action-widgets` | 663 | 1100 | 438 |
| … last widget slot | 1026 | 1057 | 31 |
| … "More" button (`padding: 2px 8px`) | 1061 | 1100 | 40 |
| window controls | 1106 | 1190 | 84 |

- The widgets sit 4 px apart (`.action-widgets { gap: var(--space-1) }`), and each slot is about 31 px for an 18 px icon.
- Between the More button and the controls there are 6 px of `.system-status` gap. The More button's own 8 px right padding comes before that.
- The three `.action-widgets--measure` copies inside `SystemStatus` are `position: absolute` and `visibility: hidden`, so they add no space. I checked, because a hidden copy in the flow would have been an obvious cause.

### 2.3 Where the gap comes from

- **Windows, and Linux before #5039:** More's 8 px padding + 6 px gap + 18 px of empty tile before the 10 px Minimize glyph (`window-controls.win32.scss`: 46 px tiles, glyph centred) is about **32 px of empty space** between the More chevron and the Minimize glyph. That is nearly twice the ~17 px between two widget icons. This is the gap the operator sees.
- **Linux since #5039 (GNOME-style controls, merged 2026-10-16):** the controls are round buttons with a visible fill, so the eye measures to the circle's edge, about 14 px (8 + 6). That's smaller, but still not the widgets' rhythm.
- The 46 px tile with its 18 px inset is the native Windows 11 caption button, and it should stay. The space that is ours to fix is the 6 px gap and where the grip sits.

## 3. Proposal

### 3.1 Placement

- **Windows and Linux:** render the grip inside `SystemStatus`, between `ActionWidgets` and `WindowControlsRight`:
  ```
  [tabs … fill] [widgets] [grip] [– ▢ ×]
  ```
- **macOS:** render it after `SystemStatus` and before the hamburger:
  ```
  [● ● ●] [tabs … fill] [widgets] [grip] [☰]
  ```
- **One component in both places:** move the grip out of `tabbar.tsx` into its own small component (for example `WindowGrip` in `frontend/app/window/`), the way the pane grip was split out (`PaneGrip.tsx`). It keeps everything the grip has today: `data-drag-region="true"` (drag, double-click to maximize, right-click for the title-bar menu), the dots taking no events, `aria-hidden`, the default arrow cursor, the dim dots that brighten on hover, and the header's bottom hairline under it.
- **Still outside the scroll:** the grip remains outside the scrolling tab strip, so it never scrolls or fades (§5.8 holds). The tab strip's empty `.tab-bar-fill` stays draggable as it is now.
- **When the widgets collapse:** at narrow widths the widget bar folds icons into More (`top-bar-tier.ts`). The grip sits after the whole widget bar, so it stays next to the controls at every width. It should be counted as fixed width in the tier calculation, as it is today while it sits in the tab bar.

### 3.2 The gap

- **Grip width:** make the grip the width of a widget slot (about 31 px instead of 32), with its 16×12 dots centred, so it reads as one more item in the row.
- **Gaps around the grip:** use `var(--space-1)` (4 px), the widgets' own gap, instead of `.system-status`'s 6 px. The spacing from the last widget to the grip to the controls then matches the spacing between widgets.
- **Windows:** keep the 46 px caption tiles as they are, since that inset is the native look. With the grip in between, the empty space is now dots followed by the native tile, instead of one blank stretch.
- **Linux (GNOME-style):** the GNOME controls group keeps its own 6 px spacing between its circles, which is GNOME's.
- **macOS:** apply the same 4 px gap on both sides of the grip before the hamburger.

### 3.3 What doesn't change

- The pane header grip (before the pane controls).
- Window dragging and double-click behaviour, the right-click menu, and the hairline.
- The widget bar's contents and order.

## 4. Spec update

`SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md` §5.8 says the window grip sits "just left of the widgets". If this proposal is taken, §5.8 gets an amendment with this placement and the operator's words, and §5.1's "Where" points to it.

## 5. Tests and checks

- **vitest:** the grip renders inside `SystemStatus`, between the widget bar and the window controls, on Windows and Linux. On macOS it renders after `SystemStatus` and before the hamburger. The tab bar no longer renders it, and it still carries `data-drag-region="true"`.
- **Tier test:** `top-bar-tier.test.ts` still folds widgets into More at the same widths, with the grip counted as fixed.
- **Live, each platform:**
  - The grip sits where described.
  - Dragging it moves the window, double-clicking it maximizes, and right-clicking opens the title-bar menu.
  - The spacing between the last widget, the grip and the controls looks even.
  - Coverage: Linux on charlie, Windows and macOS by a Windows and a Mac tester.

## 6. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | Placement as in §3.1 (Windows/Linux before the controls, macOS before the hamburger) | yes, as asked |
| D2 | Even 4 px gaps around the grip, the widgets' own rhythm | yes |
| D3 | Keep the Windows 11 caption tiles' native 46 px width | yes |
