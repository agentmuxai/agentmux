---
type: patch
---

fix(bashwrap): keep CONIN writer alive until after child.wait() — prevents CTRL_C_EVENT exit 130 on Windows
