---
type: patch
---

Hardening: a Windows srv started with a bare environment gives itself and its children a SystemDrive, so the shell no longer has a literal %SystemDrive% folder to make (#2524, not yet reproduced)
