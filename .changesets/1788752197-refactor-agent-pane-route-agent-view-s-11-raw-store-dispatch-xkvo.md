---
type: patch
---

refactor(agent-pane): route agent-view's 11 raw store dispatches through its AgentPaneModel handle (A9 of #1549) — post-unmount dispatches now drop safely instead of throwing, and all pane commands hit the crash-trail
