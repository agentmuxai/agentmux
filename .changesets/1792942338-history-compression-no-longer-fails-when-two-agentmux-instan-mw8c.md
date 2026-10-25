---
type: patch
---

History compression no longer fails when two AgentMux instances compress the same history store at once: a busy store ends the pass and the next one carries on, and each frame leaves the lock free for the other instance.
