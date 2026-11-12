---
type: patch
---

A container agent's publish check is switched on only where the host's gh-agent can judge a push, and a push from a container whose AgentMux is older than the check says so instead of passing silently
