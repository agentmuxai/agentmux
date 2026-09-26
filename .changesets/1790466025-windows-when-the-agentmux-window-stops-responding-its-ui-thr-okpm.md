---
type: patch
---

Windows: when the AgentMux window stops responding (its UI thread misses two liveness checks in a row, 1-2 minutes), the launcher now saves a small diagnostic dump of it to %LOCALAPPDATA%\CrashDumps\agentmux-host-hang (newest 5 kept), so a hang that has to be killed leaves evidence of its cause. Nothing is killed automatically.
