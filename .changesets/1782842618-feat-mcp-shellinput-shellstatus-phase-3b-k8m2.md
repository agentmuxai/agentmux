---
type: minor
---
feat(mcp): ShellInput + ShellStatus — Phase 3b of persistent shell

Agents can now write to a running shell's stdin (`ShellInput(shell_id, text)`) to answer interactive prompts like `Terminate batch job (Y/N)?`, and query whether a shell is still running with its exit code and line count (`ShellStatus(shell_id)`). Closes item 2 (Phase 3b) in the long-running shell tracking issue.
