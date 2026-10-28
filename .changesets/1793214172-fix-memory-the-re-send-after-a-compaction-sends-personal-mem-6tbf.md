---
type: patch
---

fix(memory): the re-send after a compaction sends Personal Memory newest first, caps each file at 8 KB, names the files it could not fit, and nudges the agent to trim large or stale files
