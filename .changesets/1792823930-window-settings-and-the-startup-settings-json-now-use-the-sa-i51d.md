---
type: patch
---

Window settings and the startup settings.json now use the same file srv reads: an AGENTMUX_HOME_OVERRIDE instance no longer reads the real home's settings, and on stable the file the app creates is the one that takes effect.
