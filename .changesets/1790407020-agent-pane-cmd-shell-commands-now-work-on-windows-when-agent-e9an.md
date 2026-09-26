---
type: patch
---
Agent pane: `!cmd` shell commands work on Windows again. An AgentMux launched from the Start menu or Explorer failed every `!cmd` with "shellexec: spawn failed" (Git Bash's `sh` is now located instead of assumed to be on PATH), and once `sh` was found every command hung until the 5-minute timeout (the shell no longer inherits the server's stdin). Also: `  !cmd` with leading spaces is highlighted like `!cmd`, and the shell drawer opens at 80% of its previous default height.
