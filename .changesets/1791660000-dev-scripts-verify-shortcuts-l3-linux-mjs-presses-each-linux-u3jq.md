---
type: patch
---

Dev: scripts/verify-shortcuts-l3-linux.mjs presses each Linux shortcut key through ydotool and reports whether it reaches AgentMux or the desktop takes it first, skipping the keys GNOME binds and refusing to run unless the AgentMux window has keyboard focus.
