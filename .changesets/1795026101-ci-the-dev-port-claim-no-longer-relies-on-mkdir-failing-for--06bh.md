---
type: patch
---

ci: the dev port claim no longer relies on mkdir failing for the second caller (uutils mkdir, Ubuntu 26.04, let several picks take one port), which made vite-port.test.sh fail at random
