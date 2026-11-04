---
type: patch
---

srv: the checks and plans a shell pane makes before its PTY opens (connection available, mock connection, close on exit, memory gate, WSL and SSH) are their own functions, out of ShellController::start (no behaviour change)
