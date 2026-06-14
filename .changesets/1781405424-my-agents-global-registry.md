---
type: patch
---

fix(agents): surface cross-channel agents in "My Agents". Fix the live registry mirror to anchor on the GLOBAL workspace root (the live-write twin of #1393 — newly-created agents were silently dropped as "not representable"), and source the My-Agents list from the global registry (deduped by definition+name, enriched with local running state, local-only agents appended) so agents created in any build/channel/version appear.
