---
type: patch
---

A container agent's push to a public GitHub repository is checked by the host's publish guard before it is sent (the push's commit messages, added paths and added lines go to srv as data; nothing on the host reads the container's clone)
