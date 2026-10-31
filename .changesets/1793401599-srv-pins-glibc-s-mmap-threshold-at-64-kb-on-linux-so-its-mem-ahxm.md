---
type: patch
---

srv pins glibc's mmap threshold at 64 KB on Linux, so its memory stays flat over a long session with many agents (12 agents, 60 rounds: 58.0 MB instead of 109.6 MB)
