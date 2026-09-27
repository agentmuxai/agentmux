---
type: patch
---

perf(agents): install the pinned CLI of every provider your agents use in the background at startup, so the first open after a CLI pin bump doesn't wait on npm
