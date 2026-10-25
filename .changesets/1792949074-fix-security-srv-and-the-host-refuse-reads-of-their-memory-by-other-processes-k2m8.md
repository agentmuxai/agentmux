---
type: patch
---

fix(security): srv and the host's main process refuse reads of their memory by other processes of the same user (Linux: marked not dumpable; macOS: debugger attach refused); dev builds, a headless srv and AGENTMUX_ALLOW_PTRACE=1 are left alone
