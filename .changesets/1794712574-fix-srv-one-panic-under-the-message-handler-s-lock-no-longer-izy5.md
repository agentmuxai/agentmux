---
type: patch
---

fix(srv): one panic under the message handler's lock no longer stops all agent messaging until a restart; the build-slot idle note is delivered inside the runtime
