---
type: patch
---

Fix tab moves and layout writes that could leave the backend's in-memory state and the database out of step: a workspace can no longer be emptied by two tab moves at once, focus/magnify changes from layout edits are saved, and whole-row object writes can no longer overwrite workspace, tab, block or window structure
