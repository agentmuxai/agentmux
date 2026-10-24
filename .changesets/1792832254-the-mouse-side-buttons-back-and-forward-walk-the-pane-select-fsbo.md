---
type: patch
---

The mouse side buttons (Back and Forward) walk the pane selection history over the app's own UI on Linux and macOS: the action ran on a mouse release that Chromium drops after a cancelled pointer press, so it never fired.
