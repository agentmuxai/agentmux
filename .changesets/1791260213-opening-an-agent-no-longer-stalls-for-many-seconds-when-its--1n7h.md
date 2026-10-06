---
type: patch
---

Opening an agent no longer stalls for many seconds when its new session starts writing during the history read; updating the conversation index appends new entries instead of rewriting it, so opens of long-running agents are faster
