---
type: patch
---

Settings › Agents no longer refuses to update agent ids because of a stale srv.lock: a lock whose process id now belongs to another program isn't taken for an older AgentMux running
