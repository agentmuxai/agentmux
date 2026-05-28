---
type: patch
---

fix(cef): rate-limit renderer_terminated log on the crash target (100ms gap, suppressed_count rolled forward)

The 2026-05-28 incident wrote 884 MB of host log in 22 minutes —
139,205 identical `renderer_terminated` lines on the `crash` target.
The synchronous file write on the UI thread is what produced the
user-visible "input is hard" symptom (the live renderer's IPC was
starved by host CPU/IO).

The per-browser crash budget added earlier already caps the
worst-case to ~3 events per browser per 10 s, but defense-in-depth:
add a process-wide rate limit on the same event so a fan-out scenario
(many browsers all crashing simultaneously) can't reproduce the
log-volume failure mode even before per-browser budgets trip.

Two `static AtomicU64`s track the last-emitted timestamp and a
suppressed counter. Events arriving within 100 ms of the last logged
event are silently counted; the count is emitted as
`suppressed_since_last` on the next un-throttled event, so no
information is lost — only the duplicate volume is.

Intentionally NOT a full custom tracing Layer: the in-place check is
~15 lines, zero allocation per crash, no new module, and targeted at
the only event known to spam. A generic rate-limit Layer would be
nicer ergonomically but is much more code and currently solves
nothing the per-browser budget + this in-place throttle don't already
solve.

Closes the last actionable item of #1117.
