---
type: patch
---

Agent opens no longer spawn the CLI's --version every time: the version is asked of each CLI file once, shared by agents opening together, and a timed-out probe is not repeated for 2 s
