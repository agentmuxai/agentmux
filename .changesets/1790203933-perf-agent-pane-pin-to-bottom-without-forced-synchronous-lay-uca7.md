---
type: patch
---

perf(agent-pane): pin-to-bottom without forced synchronous layout — the pin runs after layout in the content ResizeObserver, the scroll event our own pin causes is handled without geometry reads, rows are placed from the stored scroll margin
