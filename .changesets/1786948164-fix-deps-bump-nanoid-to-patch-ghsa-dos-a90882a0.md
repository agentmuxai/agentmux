---
type: patch
---

fix(deps): bump nanoid 3.3.16 -> 3.3.18 (GHSA, high severity DoS)

Custom generators in nanoid < 3.3.18 can loop indefinitely when size is
zero. Transitive-only (via postcss), no code changes needed.
