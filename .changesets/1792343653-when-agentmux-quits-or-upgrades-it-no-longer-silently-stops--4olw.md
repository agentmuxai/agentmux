---
type: patch
---

When AgentMux quits or upgrades it no longer silently stops what agents left running (a dev server, an AgentMux an agent started): the upgrade lists it in the version panel and its dialog, another AgentMux is asked to close its windows first so it saves its session, and each agent is told afterwards what of its own was stopped
