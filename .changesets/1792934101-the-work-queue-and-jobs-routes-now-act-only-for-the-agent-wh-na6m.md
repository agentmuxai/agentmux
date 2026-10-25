---
type: patch
---

The work queue and jobs routes now act only for the agent whose token the request carries; a request with no token is refused instead of being taken as the operator or as whoever the body names
