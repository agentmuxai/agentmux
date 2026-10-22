---
type: patch
---

The shared transcript store can hold compressed frames, including frames that store the streaming reply deltas once per block: every read and write path understands them. Nothing creates one yet; compression comes in a later release, off until you turn it on
