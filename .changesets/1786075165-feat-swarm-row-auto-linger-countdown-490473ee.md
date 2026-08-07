---
type: minor
---

feat(swarm): auto-retire a finished row after a 60s countdown

A completed Agent Tool or Workflow row in the Swarm tree used to linger
indefinitely until manually dismissed — a finished 12-second subagent and
a finished 6-hour workflow behaved identically, cluttering the "what's
happening now" view with things that are no longer happening. A clean
terminal row (idle subagent, or a fully-completed Workflow dispatch) now
shows a live "disappearing in Ns" countdown and auto-retires at 0, reusing
the existing retire/un-retire-on-new-activity mechanism. Hovering a row
pauses its countdown so reading a just-finished row's output doesn't race
it disappearing; manual dismiss still works immediately at any point.
Failed/interrupted rows are unaffected — they still require an explicit
dismiss, so a failure never silently vanishes before being read.
