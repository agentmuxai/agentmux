---
type: patch
---

MuxBus sign-in is now per channel, and saving it is safe across processes: a read that raced a write is retried instead of signing you out, which was why MuxBus kept logging out.
