---
type: patch
---

The headless srv container image can now be built for arm64 on an x86 machine: it is cross-compiled instead of compiled under emulation, and CI smoke-tests both architectures.
