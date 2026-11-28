---
type: patch
---

Window resize: with many terminals open, a window drag draws more of its frames. The terminals' grids now spend 8% of a drag's time refitting, down from 25%, and every grid that is still behind is refitted as soon as the drag has been still for a moment, so it always ends on the exact size.
