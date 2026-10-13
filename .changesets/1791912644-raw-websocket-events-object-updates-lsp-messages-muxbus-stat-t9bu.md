---
type: patch
---

Raw WebSocket events (object updates, LSP messages, MuxBus status, device pairing) and install progress and block activity are typed broker events, and CI refuses an onMuxEvent handler written against the event.
