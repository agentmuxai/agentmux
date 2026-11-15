---
type: patch
---

srv: a panic storm while the message handler's lock is stuck no longer stops the panic reports (the note to the agents is delivered on a thread of its own)
