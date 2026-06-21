---
type: patch
---

fix(pool): paint pool-promoted windows on Windows — drive CEF Views Window.show() on the UI thread at promote (macOS parity), register the promoted HWND for chrome ops (drag/close/min/max), and stamp FileDescription/ProductName=AgentMux on the host exe so the taskbar no longer shows "CEF Bootstrap application"
