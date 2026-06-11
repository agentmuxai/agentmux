---
type: patch
---

fix(agent): remove keyboard hint line; soften input focus border

Removes the "Enter to send • Shift+Enter for newline • Esc to clear / stop"
hint line from the agent pane composer footer — it consumes vertical space
without adding value for returning users.

Changes the textarea focus border from full `--accent-color` to
`color-mix(in srgb, var(--accent-color) 40%, transparent)` — a lighter
variation of the pane-selected border color that stays visually connected
to the theme without competing with the pane focus ring.
