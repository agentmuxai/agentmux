---
type: patch
---

fix(macos): window drag and right-click context menus now coexist on the title bar

The macOS title bar used `-webkit-app-region: drag` so the OS could move the
window, but Chromium swallows every event (including `contextmenu`) on those
regions — so right-click context menus never fired on empty title-bar space,
and the only workaround broke dragging. Switch macOS to the JS-driven drag
model already used on Linux: the header stays HTCLIENT (right-click works
everywhere) and a left-button-only drag is handed to the host, which runs a
manual move loop — pumping the drag events and repositioning the window until
the button is released — with no patched libcef.
Dragging the window and right-clicking the title bar now both work on the same
surface.
