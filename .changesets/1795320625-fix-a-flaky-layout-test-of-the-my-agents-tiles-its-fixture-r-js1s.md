---
type: patch
---

Fix a flaky layout test of the My Agents tiles: its fixture read the clock once per row, so a millisecond tick reordered the rows and measured another layout
