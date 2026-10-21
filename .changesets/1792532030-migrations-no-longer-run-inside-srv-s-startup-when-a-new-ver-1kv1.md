---
type: minor
---

Migrations no longer run inside srv's startup: when a new version's data needs updating, the launcher runs it as one step ("Updating data") on the splash, then starts srv; a normal start skips the copy, snapshot and migration lock entirely
