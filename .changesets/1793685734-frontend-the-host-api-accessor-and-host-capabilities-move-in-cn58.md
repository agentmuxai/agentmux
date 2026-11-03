---
type: patch
---

Frontend: the host API accessor and host capabilities move into util, and cef-api moves under app/host, so util no longer imports from app (10 inverted imports gone)
