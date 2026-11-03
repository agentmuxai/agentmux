---
type: patch
---

The merge queue's Rust leg builds the test targets and runs the tests that read the source tree, so two PRs that only clash in such a test (#6193 and #6196) are caught before they land
