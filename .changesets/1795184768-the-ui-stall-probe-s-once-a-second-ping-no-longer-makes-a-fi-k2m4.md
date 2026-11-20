---
type: patch
---

The UI-stall probe's once-a-second ping no longer makes a file lookup on the UI thread, which could hold it for seconds, and a stall it watched is never recorded as shorter than it saw
