---
type: patch
---

The agent watchdog's max-runtime and idle limits now work for every kind of agent, not only terminal ones (idle time counts only while a turn runs). A pending-shutdown warning now gets through a notification pause, and the session archiver runs its first sweep a minute after start instead of an hour.
