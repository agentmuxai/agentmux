---
type: patch
---

A route agent on a model with a 1M window compacts at 300K tokens instead of near the end of the window, so it stops re-reading a million tokens every turn
