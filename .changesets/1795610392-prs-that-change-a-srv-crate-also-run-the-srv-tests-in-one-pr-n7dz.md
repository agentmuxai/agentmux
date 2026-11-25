---
type: patch
---

PRs that change a srv crate also run the srv tests in one process, so a test that only passes in a fresh process fails on the PR, not in the nightly
