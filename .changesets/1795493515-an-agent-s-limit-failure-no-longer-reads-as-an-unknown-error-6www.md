---
type: patch
---

An agent's limit failure no longer reads as an unknown error: the cause Claude Code writes in its own message is read when the turn's result frame names none, so the card carries the limit and its reset and clears once the reset has passed.
