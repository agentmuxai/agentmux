---
type: patch
---

A dev frontend that can't be reached no longer reloads the window in a loop or cycles pool windows: the window waits quietly, reloads when the address answers, and a dev build quits if it stays gone
