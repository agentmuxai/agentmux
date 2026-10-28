---
type: patch
---

A signed LAN message whose sender's key was missed by an earlier lookup is checked again, so it no longer arrives as unverified; each LAN send and receipt logs whether it was signed and why not.
