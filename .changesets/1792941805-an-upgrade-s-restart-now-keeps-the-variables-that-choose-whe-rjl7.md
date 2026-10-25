---
type: patch
---

An upgrade's restart now keeps the variables that choose where an instance's data lives (its root, config folder and channel), so an instance started on a root of its own comes back on that root instead of the machine's real one.
