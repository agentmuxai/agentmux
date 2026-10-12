---
type: patch
---

perf(macos): build the DMG with LZFSE instead of LZMA; launching from or copying out of a fresh mount reads the app about 10x faster (DMG 170 MB -> 223 MB)
