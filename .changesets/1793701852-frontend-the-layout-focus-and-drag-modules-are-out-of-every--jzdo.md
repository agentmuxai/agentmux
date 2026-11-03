---
type: patch
---

Frontend: the layout, focus and drag modules are out of every import cycle (the layout model is built through a factory app-init hands to layoutModelHooks, TileLayout is imported by its one user instead of the layout barrel, and the block component model map sits in its own module)
