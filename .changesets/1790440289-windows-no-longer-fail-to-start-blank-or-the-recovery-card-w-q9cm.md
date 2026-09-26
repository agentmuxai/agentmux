---
type: patch
---

App windows no longer fail to start (blank, or the recovery card) when a single request during startup is refused. Startup reads now retry a request that never got a response, a failed object fetch is re-fetched instead of being cached as missing, and a startup failure now auto-recovers (bounded reloads) instead of stopping on the card. The startup card no longer claims the host connection was lost, and a failed startup is no longer logged as loaded successfully.
