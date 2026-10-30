---
type: patch
---

A container agent whose working folder is a ~/... path (the default) has it expanded before Docker mounts it, so an isolated instance no longer fails with 'mount path must be absolute'
