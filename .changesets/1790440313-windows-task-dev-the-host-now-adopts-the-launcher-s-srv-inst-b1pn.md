---
type: patch
---

Windows task dev: the host now adopts the launcher's srv instead of starting a second srv on the same data directory. The launcher stamps AGENTMUX_LAUNCHER_PID on Windows too, and a dev host verifies it against its real parent process before trusting the launcher's backend hand-off (as on macOS/Linux).
