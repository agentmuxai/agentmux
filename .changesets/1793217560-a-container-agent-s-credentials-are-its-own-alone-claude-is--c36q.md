---
type: patch
---

A container agent's credentials are its own alone: ~/.claude is 700 in the image, and at container start the credentials file is handed to the turn user when root owns it and set to 600, with a log line saying what the host really presents
