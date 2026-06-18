---
type: patch
---

feat(ui): thin scrollbars everywhere except the agent-pane main scroll and terminal

Make half-width (7px) the default for all native scrollbars and OverlayScrollbars
(`--os-size`), and keep full-width (14px) only on the two primary reading
surfaces: the agent conversation (`.agent-document`) and the terminal (xterm's
own overlay scrollbar, already sized independently). Drops the now-redundant
hard-coded `width: 14px` from the command-palette and identity panels so they
follow the thin default.
