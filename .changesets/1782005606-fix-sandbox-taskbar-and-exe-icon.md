---
type: patch
---

fix(cef): restore the AgentMux icon on Windows sandbox builds (Chrome-icon regression from #1633). Load the window/taskbar icon from the host module (cdylib) via GetModuleHandleExW(FROM_ADDRESS) instead of the bootstrap.exe process exe, and stamp the AgentMux icon onto the bootstrap.exe host at package time with rcedit (fixes the Explorer / Task Manager / Alt-Tab exe-file icon).
