---
type: patch
---

fix(srv): resume, the agent's segment log, continuity state, archives and the snapshot mirror treat a corrupt shared transcript zone like the transcript path does: they stop using it and fall back to the channel's own copy
