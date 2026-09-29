---
type: patch
---

fix(dnd): dropped and pasted files use one copy routine: `.env` keeps its name (a second copy is `.env_1`, not `_1.env`), two drops of the same name no longer race, folder copies run off the UI thread, and container-pane paste gives the same notices and @mentions as a drop
