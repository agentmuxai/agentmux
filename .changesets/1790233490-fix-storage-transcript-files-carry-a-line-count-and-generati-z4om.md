---
type: patch
---

fix(storage): transcript files carry a line count and generation, so every record gets a stable address; torn last lines are closed instead of fusing with the next record; concurrent instances can open the transcript store at the same moment
