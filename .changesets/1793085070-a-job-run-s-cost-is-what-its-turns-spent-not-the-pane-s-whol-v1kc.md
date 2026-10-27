---
type: patch
---

A job run's cost is what its turns spent, not the pane's whole session: Claude's total_cost_usd counts the session, so a job delivered into a long-lived pane no longer reads the session's spend as its run's and stops at its cap
