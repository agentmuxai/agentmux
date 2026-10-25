---
type: patch
---

Checking for updates no longer fails with 403 when this computer has used GitHub's hourly allowance for anonymous requests: the latest release is read from its web page, the details are asked for only when there is something new, and if GitHub still limits the check the message says when to try again. The Kanban's PR check also asks less of that allowance.
