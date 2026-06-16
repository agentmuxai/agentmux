---
type: minor
---

refactor(agent-pane): remove the per-row hover strip (timestamp + expand button). Expand/collapse now lives on each surface's own header + the row keyboard handler; section headers and activity-log lines became click-to-toggle. Per-line hover timestamps are dropped.
