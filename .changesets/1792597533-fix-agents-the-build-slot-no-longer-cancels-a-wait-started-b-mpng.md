---
type: patch
---

fix(agents): the build slot no longer cancels a wait started by cmd.exe, PowerShell or task.exe, which made a cargo that had to wait for the slot exit 129 without running
