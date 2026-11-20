---
type: patch
---
A container agent's dock no longer keeps stale background-task rows: they end when its container is started again, and a row nobody can vouch for reads "seen X ago" and leaves the dock 10 minutes after it turned unknown
