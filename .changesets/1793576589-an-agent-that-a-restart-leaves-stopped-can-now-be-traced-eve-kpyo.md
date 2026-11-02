---
type: patch
---

An agent that a restart leaves stopped can now be traced: every stop of an agent's desired state logs the code that decided it, the startup backstop logs why it leaves an agent alone, and the restart resume always logs once
