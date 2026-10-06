---
type: patch
---

Agent pane: the context meter shows the real context size and the model's real window. A freshly opened pane could show "17m / 200k"; it now reads the last API call's prompt against the window Claude Code reports (1M for Sonnet 5.5). After a compaction the meter waits for the next reply instead of showing a far-too-small number, a model switch blanks the window until the new model replies, and the meter no longer resets after a slash command, an archive/restore or a failed memory reinjection.
