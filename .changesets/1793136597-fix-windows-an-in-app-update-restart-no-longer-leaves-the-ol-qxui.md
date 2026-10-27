---
type: patch
---

fix(windows): an in-app update restart no longer leaves the old AgentMux running when a window is still shown during the quit; the quit watchdog hides it and gives up re-arming after five cycles
