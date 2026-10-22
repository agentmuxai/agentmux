---
type: patch
---

New setting agent:transcriptframes (off by default) compresses the finished parts of agent transcripts in the shared transcript store, about 6x smaller, with nothing a reader sees changing; it waits while an older AgentMux build that can't read compressed history is running
