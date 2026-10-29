---
type: patch
---

CI: the required Ubuntu Rust leg no longer runs a separate cargo check before its nextest build (about 2.5 min off every PR); the ts-rs serde check reads that build's warnings
