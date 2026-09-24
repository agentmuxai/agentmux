---
type: patch
---

fix(storage): transcript writes are atomic — two AgentMux instances appending for one agent no longer overwrite each other's lines, and a failed write no longer leaves a torn file
