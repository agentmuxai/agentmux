---
type: patch
---

fix(launcher): on macOS, a Quit from AppKit (Dock, logout, System Settings' Quit & Reopen) now runs the launcher's teardown of the host and srv instead of exiting on the spot
