# Tab tear-off — research report on cross-platform best practices

**Created:** 2026-05-07
**Owner:** AgentA
**Context:** PR #730 ships position + threshold fixes; user wants Chrome-style "see the full tab content as you drag" cross-compatible with Linux + macOS

## TL;DR

**No single "native" mechanism works on all three platforms.** The most successful production implementations converge on a **bitmap-snapshot drag image** approach: capture the dragged content as an image at drag-start, hand it to the OS as the HTML5 drag preview, and materialize the real window on drop. This is what Chromium itself uses on Wayland (where its X11/Win32 native paths can't work) and what every Electron-based app does.

Live cursor-following windows (the "follow-cursor at full opacity" experience) are **always platform-specific custom drag loops** — never native OS handoff. WinUI 3 builds one with `GetAsyncKeyState` + timer; macOS uses `NSWindow.performWindowDragWithEvent`; Linux requires X11-specific protocols. Each is bespoke.

For AgentMux (CEF on Win/macOS/Linux), the recommended path is the bitmap-snapshot approach. It's what the user's request ("see the full tab as you drag") actually maps to in cross-platform reality.

## What each major app/framework actually does

### Chromium (the real implementation)

Has THREE separate code paths in `TabDragController`:

1. **Win32 / X11 with global screen coords** — native modal-drag loop using SetCapture / equivalents. The TabDragController synthesizes mouse events into a "drag with the cursor" loop that calls SetWindowPos per frame. This is what "the new window follows the cursor at full opacity" actually means in Chrome's source: a custom drag loop in C++, not OS-native handoff.

2. **macOS** — uses `NSWindow performWindowDragWithEvent:` which is macOS's native equivalent of SC_MOVE; takes a button-down NSEvent and enters Cocoa's modal drag. Works because macOS exposes the drag entry point cleanly to apps.

3. **Wayland (fallback)** — bitmap-snapshot DnD. Chromium can't get global cursor coords or position windows directly on Wayland, so it falls back to: regular HTML5-style DnD session with the dragged tab content rendered as a bitmap drag icon. On drop, a new window materializes wherever the cursor was. This is the "see the snapshot during drag" pattern.

**Key takeaway:** even Chromium itself, which shipped tab tear-off years ago, doesn't have a unified cross-platform "native" path. The Wayland fallback is the most portable; the others are platform-bespoke.

Source: [Igalia blog — Implementing fallback tab dragging for Wayland in Chromium](https://blogs.igalia.com/max/fallback-tab-dragging/), [chromium/src TabDragController](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/views/tabs/browser_tab_strip_controller.cc)

### WinUI 3 (modern Win32 frameworks have OLE-capture issues — same as us)

The Dev.to author hit exactly the problem we're hitting:

> "WinUI 3's IXP layer filters NC messages. Sending `SC_DRAGMOVE` results in **nothing happening**."

Their solution: a **custom drag loop** with:
- `DispatcherTimer` polling at 120Hz / 8ms intervals
- `GetAsyncKeyState(VK_LBUTTON)` to read button state directly (bypasses framework message-queue filtering)
- `SetWindowPos` per tick to follow cursor
- `DWM cloak` to hide window during init flicker
- 30-frame grace to prevent instant re-docking

This is path-2 from yesterday's plan. The author confirms: **no shortcut exists in modern Win32 frameworks** — you MUST build the drag loop yourself.

Source: [Implementing Chrome-Style Tab Tear-off in WinUI 3 — DEV Community](https://dev.to/nwlsrb/implementing-chrome-style-tab-tear-off-in-winui-3-3k3j)

### VS Code (Electron)

VS Code shipped tear-off in 2023 (after a 7-year-old GitHub issue). Implementation:
- HTML5 drag-and-drop within a window (in-pane reordering)
- For cross-window drag, custom IPC between BrowserWindows because HTML5 drag is single-context
- Visual: HTML5 native drag image (the OS-rendered ghost)
- On drop outside any window, Electron creates a new BrowserWindow at the cursor position

VS Code does **not** have follow-cursor live windows — the new window appears on drop, identical to AgentMux's current behavior post-PR #730.

Source: [microsoft/vscode#53984 (drag tab to create new window)](https://github.com/microsoft/vscode/issues/53984), [Electron Tutorial: How to Drag Tabs Between Open Windows](https://www.codestudy.net/blog/electron-drag-tab-into-another-open-window/)

### JetBrains IDEs (IntelliJ / WebStorm / etc.)

Editor tabs detach via mouse drag, with `Shift+F4` as keyboard alternative. Built on JetBrains Runtime (OpenJDK fork) with JCEF for embedded browsers.

Implementation is **Swing-based** — JetBrains' Java UI handles the drag via Swing's `DragSource` / `DropTarget` APIs, which in turn map to platform-native drag protocols. The IDE shows a translucent ghost during drag (Swing's default), creates the floating editor window on drop.

Source: [JCEF documentation](https://plugins.jetbrains.com/docs/intellij/jcef.html)

### Tauri / Wails (newer cross-platform desktop frameworks)

- Tauri's window-drag API (`startDragging()`) supports moving the existing window, not detaching content into a new one.
- Cursor capture (`grabCursor`) is platform-uneven: unsupported on Linux; on macOS it locks the cursor at a fixed position (visually awkward).
- No first-class tab-tear-off API. Apps building tear-off on Tauri have to implement it themselves and report platform inconsistencies.

Source: [Tauri v2 window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)

### Vivaldi / Brave / Edge (Chromium forks)

All inherit Chromium's TabDragController unchanged → same three platform paths described above.

## Cross-platform best practice: the bitmap-snapshot pattern

What every successful cross-platform implementation actually does (or falls back to):

1. **At drag-start**: capture the source content (the dragged tab's rendered area) as a bitmap.
2. **Hand the bitmap to the OS as the HTML5 drag image** via `dataTransfer.setDragImage()` or `setCustomNativeDragPreview` (pragmatic-dnd).
3. **OS renders the bitmap during drag** — this is the "see the full tab" visual the user wants.
4. **On drop outside any drop zone**: the new window materializes at cursor position.

Trade-offs vs. the live-window approach:
- ✓ Works identically on Win/macOS/Linux/Wayland (HTML5 drag is the common substrate)
- ✓ No OLE-capture / SC_MOVE / GetAsyncKeyState platform-specific gymnastics
- ✓ ~half a day of work, not 2-3 days × 3 platforms
- ✗ Snapshot is static (terminal stops scrolling in the preview during drag)
- ✗ Position lag: bitmap follows cursor, real window appears on release. User who drags slowly sees "ghost then jump to drop position" — but the position is now correct (PR #730 fix).
- ✗ Tabs with `<iframe>` / `<canvas>` content snapshot poorly with DOM-based libs

## Library comparison for the bitmap step

For DOM-to-bitmap, the library landscape (May 2026):

| Library | Speed (10 widgets) | Modern CSS | Maintained | Notes |
|---|---|---|---|---|
| `html2canvas` | ~21s (slowest) | poor flexbox/grid | yes | The classic — most documented, but slow |
| `dom-to-image` | medium | broken on modern CSS | **no** | Dead since 2023 |
| `html-to-image` | medium-fast | good | yes | Active fork of dom-to-image |
| `modern-screenshot` | ~7s (3× faster than html2canvas) | best | yes | Newest, best perf, fork of html-to-image |

**Recommendation:** `modern-screenshot`. Best perf, most modern, smallest API surface.

For our case, "10 widgets in 7s" sounds slow but each tab snapshot is a single capture, not 10. Real-world capture of one tab's rendered area should be ~50-150ms, comfortably below the user's perceptual threshold.

Source: [npm-compare html2canvas vs modern-screenshot vs others](https://npm-compare.com/html2canvas,modern-screenshot,puppeteer,screenshot-desktop), [monday.com engineering — capturing DOM as image](https://engineering.monday.com/capturing-dom-as-image-is-harder-than-you-think-how-we-solved-it-at-monday-com/)

## Recommendation for AgentMux

### Path A (recommended — single PR, ~½ day)

**Bitmap-snapshot drag preview, cross-platform via HTML5 drag.**

- Add `modern-screenshot` to package.json
- In `droppable-tab.tsx::onGenerateDragPreview`:
  - Capture the active tab's content area (the `LayoutModel`'s root DOM node) as a bitmap
  - Render the bitmap into the `setCustomNativeDragPreview` container
  - Position so cursor lands on the tab portion (offset already captured by PR #730)
- Optionally: capture starts on `mousedown` (before drag) to hide the 50-150ms snapshot latency

This gets the user the "see the full tab as you drag" they asked for, works on all three platforms, doesn't touch the existing tear-off-on-release flow, and keeps PR #730's wins.

### Path B (deferred — separate large project)

**Custom drag loop per platform** (Chrome's actual implementation pattern).

- Win32: `mousedown` → `SetCapture` → `mousemove` → IPC → `SetWindowPos` per frame → `mouseup` → release
- macOS: `mousedown` → `NSWindow performWindowDragWithEvent`
- Linux X11: `mousedown` → `_NET_WM_MOVERESIZE`
- Linux Wayland: HTML5 DnD with bitmap (= Path A on Wayland)

Why defer: 2-3 days × 3 platforms = ~weeks of work, with maintenance cost forever. Three separate state machines. No clear win over Path A for AgentMux's use case.

The only thing Path B provides that Path A doesn't: **live content during drag** (the tab's content keeps animating in the preview). For terminals / agent panes this might matter, but for static editor content it's identical.

### Combined recommendation

Ship Path A in a follow-up to #730. Treat Path B as a separate project — possibly re-evaluate after Path A ships and we see whether the static snapshot is sufficient.

## Open questions

1. **Will `modern-screenshot` capture our terminal panes (xterm.js canvas)?** Canvas content can be captured with `canvas.toDataURL()` directly; need to verify the lib handles this. If not, terminal tabs would show as empty in the preview.
2. **What's the snapshot latency on a 4K display with a busy DOM tree?** Need a quick spike. If >300ms, capture on `mousedown` is required.
3. **How does this interact with Chrome's own drag image suppression on certain elements?** Need to verify `setCustomNativeDragPreview` actually replaces the OS default in our CEF version.

## Spike to validate before full impl

15-minute spike: install `modern-screenshot`, snapshot the active tab's content on a button click, render to a `<img>` tag, time it. If <200ms on a typical workspace, ship Path A. If not, capture on `mousedown` or fall back to lower-resolution snapshot.

## Sources

- [Implementing Chrome-Style Tab Tear-off in WinUI 3 — DEV Community](https://dev.to/nwlsrb/implementing-chrome-style-tab-tear-off-in-winui-3-3k3j)
- [Igalia: Implementing fallback tab dragging for Wayland in Chromium](https://blogs.igalia.com/max/fallback-tab-dragging/)
- [Tab Strip Design (Mac) — chromium.org](https://www.chromium.org/developers/design-documents/tab-strip-mac/)
- [Electron Tutorial: How to Drag Tabs Between Open Windows](https://www.codestudy.net/blog/electron-drag-tab-into-another-open-window/)
- [VS Code: Allow dragging a tab to create a new window (#53984)](https://github.com/Microsoft/vscode/issues/53984)
- [JCEF documentation — JetBrains](https://plugins.jetbrains.com/docs/intellij/jcef.html)
- [Tauri v2 window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)
- [npm-compare: html2canvas vs modern-screenshot vs others](https://npm-compare.com/html2canvas,modern-screenshot,puppeteer,screenshot-desktop)
- [monday.com engineering: Capturing DOM as image is harder than you think](https://engineering.monday.com/capturing-dom-as-image-is-harder-than-you-think-how-we-solved-it-at-monday-com/)
- [chromium/src TabDragController source (browser_tab_strip_controller.cc)](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/views/tabs/browser_tab_strip_controller.cc)
