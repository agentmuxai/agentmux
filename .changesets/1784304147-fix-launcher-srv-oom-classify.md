---
type: patch
---

fix(launcher): wire the existing commit-aware OOM retry into srv exits, not just CEF-host exits — a srv crash-loop under system OOM previously burned the fast restart budget and killed the whole launcher instead of waiting out the transient pressure
