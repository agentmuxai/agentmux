---
type: patch
---

Agents' cargo builds now really take the shared build slot, so only one heavy compile runs at a time on a machine (a cargo earlier on the PATH used to skip it), and a machine can allow two
