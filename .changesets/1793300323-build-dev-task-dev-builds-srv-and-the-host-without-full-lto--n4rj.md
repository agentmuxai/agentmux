---
type: patch
---

build(dev): task dev builds srv and the host without full LTO (no LTO, 16 codegen units, opt-level 1, incremental), so a dev build is fast and light on the machine; package and release builds are unchanged
