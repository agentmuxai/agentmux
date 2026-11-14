---
type: patch
---

Internal: the container listener's router tests and the Docker listener and turn checks move from backend::container to the server's tests, since they build the whole router (prep for the srv-core split)
