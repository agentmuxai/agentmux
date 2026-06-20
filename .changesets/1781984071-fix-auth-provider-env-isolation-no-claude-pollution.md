---
type: patch
---

fix(auth): provider environment isolation — agents read/refresh credentials in the AgentMux dir, never the user's ~/.claude (provider-isolation auth half: migration reversal of #983 pointer-to-ambient + sweep + hardened seed target)
