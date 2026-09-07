---
type: minor
---

refactor(agent-pane): drop the AgentAtoms mirror — the pane model exposes the reducer state and document nodes reactively (A6 of #1549); adding a reducer field is now a one-place change, and the two detailsOpen writes that bypassed the reducer go through it
