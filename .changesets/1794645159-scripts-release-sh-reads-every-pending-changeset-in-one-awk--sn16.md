---
type: patch
---

scripts/release.sh reads every pending changeset in one awk process instead of two per file, so a release with 150 changesets no longer spends minutes starting processes on Windows (output unchanged, pinned by a new byte-for-byte test)
