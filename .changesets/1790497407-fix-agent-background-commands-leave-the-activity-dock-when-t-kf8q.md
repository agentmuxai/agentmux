---
type: patch
---

fix(agent): background commands leave the Activity Dock when they finish — including ones a subagent started, which used to stay 'running' forever — and the agent's own background commands show up there again, named by their description
