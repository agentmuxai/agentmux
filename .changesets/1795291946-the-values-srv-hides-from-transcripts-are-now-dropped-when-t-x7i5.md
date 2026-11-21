---
type: patch
---

The values srv hides from transcripts are now dropped when they stop working (expired, revoked, replaced), the GitHub and AWS session values it hands an agent are hidden until they expire, every stored agent key is hidden from start-up, and a Claude sign-in file's tokens are hidden each time the file is read
