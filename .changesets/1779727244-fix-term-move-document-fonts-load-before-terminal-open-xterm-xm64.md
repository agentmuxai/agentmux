---
type: patch
---

fix(term): move document.fonts.load() BEFORE terminal.open() — xterm caches metrics at open() time (#1040 follow-up)
