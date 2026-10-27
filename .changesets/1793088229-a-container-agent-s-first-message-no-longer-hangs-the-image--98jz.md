---
type: patch
---

A container agent's first message no longer hangs: the image download and container start-up now run in the background instead of being cut off after five seconds, the pane shows the download's progress, and a download that stalls for a minute or runs over ten minutes ends with a clear message
