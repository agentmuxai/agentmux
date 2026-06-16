---
type: patch
---

fix(cef): low-memory pause page Resume button was inert (double-quoted JS string inside a double-quoted onclick attribute) — wire it via a <script> + addEventListener so OOM-paused windows can actually resume
