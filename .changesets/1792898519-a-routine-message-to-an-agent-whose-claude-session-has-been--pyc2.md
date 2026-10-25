---
type: minor
---

A routine message to an agent whose Claude session has been idle for about an hour is now held until that agent is next active, instead of waking it; pass urgent: true on Message (or name the task's #) to deliver it at once. The jekt:cold_hold_minutes setting changes the hour, and 0 turns the hold off
