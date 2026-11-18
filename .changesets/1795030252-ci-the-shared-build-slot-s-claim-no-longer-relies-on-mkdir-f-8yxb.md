---
type: patch
---

ci: the shared build slot's claim no longer relies on mkdir failing for the second caller (uutils mkdir on Ubuntu 26.04 can report success to a caller that lost the race); build-slot.sh is version 10
