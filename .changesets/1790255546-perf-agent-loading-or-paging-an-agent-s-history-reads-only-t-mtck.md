---
type: patch
---

perf(agent): loading or paging an agent's history reads only the timestamps it needs (~5 ms instead of ~0.6 s per page on large agents)
