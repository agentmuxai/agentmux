---
type: patch
---

fix(srv): cap cold-backfill subagent replay to the 200 most-recent files — an unbounded full-history replay on every pane reopen/srv restart was a live contributor to a launcher-killing crash loop
