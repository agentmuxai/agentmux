---
type: patch
---

fix(keys): shortcuts stop firing over typing - a terminal keeps readline's Alt keys, Ctrl+P and Ctrl+[ (terminal clear on Windows/Linux moves from Alt+K to Ctrl+Shift+L); text fields keep word selection; a key a pane or the editor handled no longer also runs a global shortcut; Ctrl+Shift+K asks before replacing a pane
