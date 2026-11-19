---
type: patch
---

fix: two AppImage starts at once no longer lose a channel entry: the account-wide AppImage channel record is read, changed and written under a file lock
