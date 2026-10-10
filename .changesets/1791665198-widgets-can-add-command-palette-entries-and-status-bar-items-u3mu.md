---
type: minor
---

Widgets can add command palette entries and status bar items: declare them in widget.json, and a running pane updates its items with ui.setStatusItem and hears its commands as the command event. The status bar is now a registry, and the PR dashboard sample adds both.
