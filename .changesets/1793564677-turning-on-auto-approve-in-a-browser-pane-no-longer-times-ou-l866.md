---
type: patch
---

Turning on auto-approve in a browser pane no longer times out on large pages: AgentMux reads the page's address from the browser's history instead of running the agent's eval, which scans the whole page first
