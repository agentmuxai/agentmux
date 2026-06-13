---
type: patch
---

fix(bashwrap): wrap commands in a /dev/null brace-group redirect instead of exec </dev/null — the exec form closed the child's ConPTY console input and ConPTY killed every streamed bash command with exit 130 before it ran; the group redirect gives stdin-readers EOF without firing ctrl-c (#1368)
