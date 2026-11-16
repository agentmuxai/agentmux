---
type: patch
---

A resync that replaces an agent's controller no longer holds the messages queued behind its turn: the replacement runs them, and what a stopped agent held is dropped when its block is deleted
