---
type: patch
---

Container and host agents' turn launch, env, srv address and mount mapping now sit behind one RuntimeAdapter trait, and the scattered is-container checks ask it (no behaviour change)
