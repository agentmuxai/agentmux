---
type: patch
---

The agent runtime store's test counts how many readers an event wakes, not how long it takes, so it does not depend on the speed of a shared runner
