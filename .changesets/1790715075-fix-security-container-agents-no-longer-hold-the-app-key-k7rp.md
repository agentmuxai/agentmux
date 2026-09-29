---
type: patch
---

fix(security): container agents no longer receive the app's full API key; they get a per-agent token that reaches only agent routes (messaging, work queue, their own memory) and is refused host commands and host files
