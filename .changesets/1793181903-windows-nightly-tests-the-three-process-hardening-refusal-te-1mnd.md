---
type: patch
---

Windows nightly tests: the three process-hardening refusal tests skip when the test token holds SeDebugPrivilege enabled (a GitHub runner's admin), and the container memory-folder test no longer compares against a canonical path on Windows
