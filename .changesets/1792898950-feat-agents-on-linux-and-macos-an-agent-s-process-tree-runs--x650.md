---
type: patch
---

feat(agents): on Linux and macOS an agent's process tree runs at nice 10, so its builds yield to AgentMux (agent:belownormalpriority, which was Windows-only)
