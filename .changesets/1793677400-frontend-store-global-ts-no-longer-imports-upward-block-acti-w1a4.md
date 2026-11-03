---
type: patch
---

Frontend: store/global.ts no longer imports upward (block actions, block registry, tab actions, modals, update/connectivity/sign-in events), taking 31 files out of import cycles
