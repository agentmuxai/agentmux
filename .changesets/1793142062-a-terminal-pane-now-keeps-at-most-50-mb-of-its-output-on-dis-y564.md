---
type: patch
---

A terminal pane now keeps at most 50 MB of its output on disk (the term:storedoutputmb setting changes it, 0 removes the limit), so a command that prints without end no longer fills the disk
