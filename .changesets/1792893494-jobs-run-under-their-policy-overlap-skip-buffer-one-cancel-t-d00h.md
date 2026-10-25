---
type: minor
---

Jobs run under their policy: overlap (skip, buffer one, cancel the other), a timeout, retries of infrastructure failures with backoff, concurrency groups with deploys one at a time, and at most three unattended agent runs at once; task jobs file a Kanban task, and spawn jobs, created from the Jobs tab, open a worker
