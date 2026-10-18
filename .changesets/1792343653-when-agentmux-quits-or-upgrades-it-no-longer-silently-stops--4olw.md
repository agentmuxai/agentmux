---
type: patch
---

When AgentMux quits or upgrades it no longer silently stops what agents left running (a dev server, an AgentMux an agent started): the upgrade lists it in the version panel and its dialog, on Windows another AgentMux among it saves its windows first, as at a shutdown, so they all come back on its next start, and each agent is told afterwards what of its own was stopped
