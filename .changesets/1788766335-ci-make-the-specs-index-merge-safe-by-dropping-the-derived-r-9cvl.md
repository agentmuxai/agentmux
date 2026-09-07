---
type: patch
---

ci: make the specs index merge-safe by dropping the derived row count from its committed section headers (the one value that resolved wrongly when two spec-adding PRs merged, and 67% of recent CI failures); fix the create_no_window_flag_set flake by pre-warming node.exe outside the measured window
