---
type: patch
---

The slug-migration crash tests run as one test per step and crash point instead of two long loops, so they run in parallel and stop hitting the test runner's kill time on Windows
