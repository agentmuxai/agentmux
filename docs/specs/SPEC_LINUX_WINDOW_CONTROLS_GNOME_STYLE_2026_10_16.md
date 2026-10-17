# Spec: Linux window controls that look like Linux

**Date:** 2026-10-16
**Author:** Maricon@charlie
**Type:** Spec (the app window's title-bar buttons on Linux)
**Status:** proposed
**Decisions applied (operator: "proceed with best recommendations", 2026-10-16):** D1 yes, D2 one GNOME style, D3 not now, D4 no, D5 later, D6 match a native app on charlie.
**Requested by:** the operator: *"we want to get linux-like window controls ...currently it adopts the windows, but for linux we want something that looks linux, should also be theme aware if applicable ... this is the overall app window, different from the floaters, but still related"*
**Related:** `SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16.md` (the floater's controls, written the same day; it shares the "show Restore when maximized" need, §3.3 here).

## 1. Summary

On Linux the app window's title bar shows the Windows 11 caption buttons: 46 px wide flat rectangles, a red Close on hover. On a Linux desktop that looks foreign. This spec gives Linux its own controls, in the style of GNOME (the default desktop on Ubuntu and Fedora, and charlie's):

- **The look:** round buttons with a soft round background behind 16 px symbolic glyphs, and a neutral Close (no red).
- **Placement and set:** the buttons follow the desktop's own button-layout setting, so they appear on the side and in the order the user's other windows use.
- **Maximized:** Maximize turns into Restore while the window is maximized.
- **Window not focused:** the buttons dim, as GNOME's do.
- **Theme:** colours come from the AgentMux theme (light and dark), so they always match the title bar they sit on.

Windows and macOS are unchanged.

## 2. What the code does today (main at `4e8868d26`)

- **The platform choice:** `frontend/app/window/window-controls.platform.tsx` is resolved at build time to `window-controls.{win32,darwin,linux}.tsx`. The Linux file is one line that re-exports the Windows components: *"Linux mirrors Windows ... the Win11-style caption buttons render on the right."*
- **The Windows buttons** (`window-controls.win32.tsx` + `.scss`):
  - 10 px Fluent glyphs drawn as inline SVG, in 46 px × full-height tiles that reach the window's top and right edges.
  - Hover and press use a neutral overlay. Close hover is solid Windows red (`--window-close-hover-bg: #c42b1c`).
  - The tint already follows the AgentMux theme's polarity (`[data-theme-polarity="light"]`), not the OS setting.
- **Where they mount:** `WindowControlsRight` at the end of `SystemStatus` (`system-status.tsx`), and `WindowControlsLeft` at the start of the window header (`window-header.tsx`). That is the hook macOS uses for its traffic lights, so a left-side placement already has a home.
- **The header** is 33 px tall on Linux (`window-header.linux.scss`).
- **Double-click on the header** toggles maximize (`useWindowDrag.linux.ts`).
- **No maximized state:** the Maximize button always shows the maximize square and calls `getApi().maximizeWindow()`, which toggles. The page has no way to know whether the window is maximized. The host has the hook: `on_window_bounds_changed` already runs on Linux for every window (`crates/cef/src/app/mod.rs`), and `window.is_maximized()` is available. This is true on every platform today, not just Linux.
- **No desktop settings are read:** nothing in the host reads GNOME's settings (button layout, colour scheme or accent). On charlie (Ubuntu 26.04, GNOME 50, Yaru-dark):
  - `org.gnome.desktop.wm.preferences button-layout` is `':minimize,maximize,close'` (all three, on the right; Ubuntu's default).
  - Stock GNOME's default is `'appmenu:close'`, which shows Close only.
  - `action-double-click-titlebar` is `'toggle-maximize'`.

## 3. The behaviour

### 3.1 The look (GNOME's title-bar buttons)

GNOME draws its window buttons (libadwaita and GTK 4 header bars, which most GNOME apps use) as:

- **Round buttons:** each control is a 24 px circle with a 16 px symbolic glyph centred in it.
- **Soft background:** the circle has a faint fill in the text colour, roughly 10% at rest, 15% on hover and 30% pressed. The glyph is the text colour.
- **Neutral Close:** it looks like the other two. No red, either at rest or on hover.
- **Spacing:** about 6 px between buttons, and a small margin from the window's edge. The buttons sit centred vertically in the bar, not full-height tiles.
- **Glyphs:** Minimize is a short bar low in the box, Maximize a square outline, Restore GNOME's restore symbol, and Close an ×. All are drawn with GNOME's stroke weight, heavier than Fluent's 1 px.

The exact numbers are to be matched against a screenshot of a native GNOME app on the same machine (Files or Text Editor) during implementation, not against these approximate values. **The glyphs are drawn for AgentMux as inline SVG, like the Windows ones, and not copied from the Adwaita icon theme**, which is licensed separately (CC BY-SA / LGPL) from this Apache-2.0 repo.

### 3.2 Placement and set: follow the desktop

The host reads the desktop's button layout once at start and hands it to the page:

- **The format:** `org.gnome.desktop.wm.preferences button-layout`, for example `':minimize,maximize,close'`. Left of the colon goes on the left, right of it on the right, in that order. Names the app does not draw (`appmenu`, `icon`, `menu`, `spacer`) are ignored.
- **Right side:** rendered by `WindowControlsRight`, as now.
- **Left side:** rendered by `WindowControlsLeft` in the header. For example, `'close,minimize,maximize:'`, the old Ubuntu Unity style, puts the buttons left.
- **Fallback:** when the setting can't be read (a desktop without GSettings, or a sandbox), use `':minimize,maximize,close'`. That is today's set and order.
- **Close-only layouts:** a layout without Close, such as `'appmenu:'`, still gets Close. An app window with no close button is a trap, and GNOME itself keeps Close in practice.
- **Changes:** a change to the setting takes effect at the next window open. Live updates are D4.

### 3.3 Maximize ↔ Restore

While the window is maximized, the Maximize button shows GNOME's restore glyph, titled "Restore". A click restores.

This needs the page to know the window's maximized state:
- The host sends it to the window's renderer when it changes: on Linux from `on_window_bounds_changed` comparing `window.is_maximized()` with the last value it sent, and once when the page loads.
- The page keeps it in a window-level signal. Double-click, Super+Up/Down, the compositor's own drag un-maximize, and the button all show up the same way.

The floater spec (§4.2 there) needs the same fact for floating panes, and keeps it in the block meta because a floater has a block. The app window has no block, so this one is an event. If this lands first, the floater's icon can use the same event (decision D2 there).

### 3.4 Window not focused

When the window loses focus, the buttons dim, as GNOME's backdrop state does. They return when it regains focus. The page's own `window` focus and blur events are enough; no host change is needed.

### 3.5 Theme

**The colours follow the AgentMux theme, not the desktop's.** The title bar is painted by AgentMux's theme, so the buttons must be legible on that surface:
- The glyph uses the theme's text colour.
- The round background is the text colour at low alpha, so the same rule works on dark and light themes (`data-theme-polarity`), with no per-theme values.
- Window transparency: the circles are translucent like the bar, so they need no special case.

**What does follow the desktop:**
- the button layout (§3.2)
- the double-click action, which is already toggle-maximize

**Not followed in this change: the desktop's colour scheme and accent colour.** Both are readable through the XDG settings portal (`org.freedesktop.appearance` `color-scheme` and `accent-color`; charlie's accent is orange). Using the desktop accent for the pressed state, or picking an AgentMux light/dark theme from the desktop's preference, is D3. Neither is required for "looks like Linux".

## 4. Design

- **`window-controls.linux.tsx`:** its own `WindowControlsLeft` and `WindowControlsRight` instead of the re-export. They render the ordered button lists from §3.2: three small components (Minimize, Maximize/Restore, Close) on the same `getApi()` calls as Windows (`minimizeWindow`, `maximizeWindow`, `closeWindow`). Each keeps `data-drag-region="false"`, the `data-testid`s (`window-minimize-btn` etc.), `title` and `aria-label`, so existing tests and automation still find them. The buttons are the shared line-style `Button` (`element/ui/`), restyled as caption buttons, because the UI ratchet (`check-ui-primitives.mjs`) allows no new hand-rolled `<button>`.
- **`window-controls.linux.scss`:** GNOME-style styling as §3.1.
  - Theme-relative colours via `currentColor` and `color-mix`.
  - A `.window-unfocused` class for §3.4.
  - It has none of the Windows file's full-height and negative-margin rules: the round buttons sit inside the 33 px bar.
- **The layout setting (host):** the Linux host reads `button-layout` once, on the first ask, through GSettings in a `libgio-2.0.so.0` loaded at runtime (`commands/desktop_settings.rs`, the way `app/gl_probe.rs` loads EGL), so there is no new build dependency and a system without GLib just gets nothing. The schema and key are looked up first, because `g_settings_new` aborts on an unknown schema. It does not shell out to `gsettings`. The page gets the raw string from one new host call (`getApi().getWindowButtonLayout()`, like `getWindowSystem()`), parses it (`frontend/app/window/window-button-layout.ts`), and loads it at startup from initGlobal on Linux.
- **The maximized state:** the page asks the host's `is_window_maximized { label }` query (added for floaters in #5037) on mount and after the window's resizes go quiet for 150 ms. A maximize or restore always resizes the window, so this needs no new host event. *(Changed while implementing: the first draft proposed a `window-placement-changed` event from `on_window_bounds_changed`.)* D1 for Windows is left to a follow-up: the query already answers there with `IsZoomed`.
- **Unchanged:**
  - the Windows and macOS files
  - the header's drag behaviour, double-click and context menu
  - the pane headers' close button, which keeps the Windows red on every platform (pane controls are app chrome, not window controls)

## 5. Tests

**Frontend (vitest, Linux build of the component):**
1. Layout `':minimize,maximize,close'` gives the three buttons on the right, in that order, and nothing on the left.
2. Layout `'close,minimize,maximize:'` gives the three buttons on the left in that order, and nothing on the right.
3. Layout `'appmenu:close'` gives Close only, on the right. Layout `'appmenu:'` still gives Close.
4. A missing or unreadable layout gives the default.
5. Maximized state true gives the restore glyph titled "Restore". False gives Maximize. A change event re-renders it.
6. Clicks call `minimizeWindow`, `maximizeWindow` and `closeWindow` once each.
7. Window blur adds the unfocused class, and focus removes it.

**Layout parsing (vitest):** empty sides, unknown names, duplicates, whitespace, a layout without Close, and nothing to read.

**Live on charlie (GNOME Wayland), then X11/XWayland:**
- Screenshot the title bar beside a native GNOME app: same shape, size, spacing and placement.
- Dark and light AgentMux themes: legible on both.
- Maximize with the button, double-click and Super+Up: the icon is Restore. Un-maximize by dragging the bar down: it returns to Maximize.
- Unfocus the window: the buttons dim.
- Change `button-layout` to the left (`close,minimize,maximize:`), reopen the window: the buttons are on the left. Then restore the original value.

## 6. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | Also send the maximized state on Windows and macOS, so the Windows caption Maximize flips to Restore there too? | yes: Windows' own caption buttons do flip, and the event is the same work; macOS's traffic lights don't need it |
| D2 | One GNOME style for every Linux desktop (KDE, XFCE, …), or detect the desktop? | one GNOME style for all, with the layout read where GSettings exists; KDE's own look (Breeze) only if asked |
| D3 | Use the desktop's colour scheme and accent colour? | not now; AgentMux's theme paints the bar, so it decides the colours |
| D4 | Pick up a `button-layout` change live, without reopening the window? | no; rare, and a reopen is enough |
| D5 | Give floaters on Linux the same GNOME-style buttons in their headers? | not in this change: a floater's header is a pane header (pin, Redock, Maximize, Close) and its sizing is being settled in the floater spec; revisit once both have landed |
| D6 | Close red on hover, like Ubuntu's Yaru theme may do? | match what a native GTK app shows on charlie at implementation time; stock GNOME is neutral |

## 7. Not covered

- Client-side window shadows, rounded window corners, and resize borders. Those belong to the window frame, not the buttons.
- The title bar's own look (height, tab strip, drag areas).
- macOS and Windows, apart from D1.
- I did not screenshot a native GNOME app for this spec. The §3.1 measurements are GNOME's published style as I know it, to be checked against the real desktop when implementing.
