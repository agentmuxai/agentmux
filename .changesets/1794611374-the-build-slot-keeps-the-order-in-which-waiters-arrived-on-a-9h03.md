---
type: patch
---

The build slot keeps the order in which waiters arrived on a slow machine: a waiter whose process is alive is no longer dropped from the line for being slow to touch its ticket, and a ticket removed by mistake is written again in its place
