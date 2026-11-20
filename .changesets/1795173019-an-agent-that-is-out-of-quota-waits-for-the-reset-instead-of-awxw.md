---
type: patch
---

An agent that is out of quota waits for the reset instead of re-sending: a 429 that names a plan window (the 5-hour limit, the weekly limit) is a usage limit whose reset time is read from the message in its own time zone, the row says when it lifts, the waiting message is sent once after the reset, and a failed turn's end no longer restarts the retry ladder
