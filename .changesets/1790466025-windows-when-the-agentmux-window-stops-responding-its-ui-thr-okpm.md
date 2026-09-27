---
type: patch
---

Windows: when the AgentMux window stops responding (its UI thread misses two liveness checks in a row, 1-2 minutes), the launcher now saves a small diagnostic dump of it to %LOCALAPPDATA%\CrashDumps\agentmux-host-hang\<instance> (newest 5 per instance), so a hang that has to be killed leaves evidence of its cause. Nothing is killed automatically.
