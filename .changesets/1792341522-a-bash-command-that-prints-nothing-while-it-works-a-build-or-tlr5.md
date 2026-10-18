---
type: patch
---

A Bash command that prints nothing while it works (a build or test run with its output in a file) is no longer killed after 10 minutes of silence; only a command that is stuck, using no CPU and starting nothing, is killed.
