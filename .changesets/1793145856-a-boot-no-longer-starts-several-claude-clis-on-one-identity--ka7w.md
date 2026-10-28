---
type: patch
---

A boot no longer starts several Claude CLIs on one identity while its token is expired: one goes first, the others and ambient calls wait for its refresh, and spawns that waited start 250 ms apart
