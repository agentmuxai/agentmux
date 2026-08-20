---
type: patch
---

fix(identity): bound keychain reads/writes with a 15s timeout so an unanswered OS consent prompt fails fast instead of hanging indefinitely
