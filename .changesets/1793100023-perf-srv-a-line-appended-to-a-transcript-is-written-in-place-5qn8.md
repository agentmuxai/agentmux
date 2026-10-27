---
type: patch
---

perf(srv): a line appended to a transcript is written in place into its tail row, so a line no longer rewrites the whole 64 KB tail part (the recovery tool knows the row's slack). srv writes about 89% fewer bytes in the memory lab. If a transcript is later appended to by an older AgentMux, its pane resyncs once
