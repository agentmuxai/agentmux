---
type: patch
---

An agent's AgentMux tools no longer stop answering for hours when one call hangs: every call has its own deadline and the others keep working, and a wedged tool server is detected and restarted
