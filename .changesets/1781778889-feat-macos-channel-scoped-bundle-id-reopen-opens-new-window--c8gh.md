---
type: minor
---

feat(macos): bundle id follows the build channel + wire the unix open_new_window forward (fixes the cross-version "not responding" dialog; `open -n`/CLI relaunch opens a new window). A kAEReopenApplication handler for plain double-click is included but currently inert — Chromium owns the event — with the NSApp-delegate swizzle tracked as a follow-up.
