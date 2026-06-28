---
type: patch
---

fix(app-api): resolve memory.* for live agents via the named-agent registry

`memory.list/read/write` resolved the agent through `instance_get_by_name`
(db_agents), but running agents are tracked in the global named-agent registry,
not as db_agents instance rows — so every live agent got "agent not found". The
resolver now falls back to the registry (slug → working_dir + identity binding)
when no db_agents row exists. Fixes #1836.
