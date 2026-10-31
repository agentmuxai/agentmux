---
type: patch
---

A container's cache directories (the developer image's cargo home and build output) are kept in per-agent volumes, so they survive an image update, and deleting a container agent removes its container and every volume made for it
