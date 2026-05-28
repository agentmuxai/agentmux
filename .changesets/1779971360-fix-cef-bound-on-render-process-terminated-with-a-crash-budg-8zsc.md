---
type: patch
---

fix(cef): bound on_render_process_terminated with a crash budget (no infinite recovery loop)

The handler in `agentmux-cef/src/client/mod.rs` historically loaded a
fresh `data:` recovery page on every renderer death with no per-browser
budget. On 2026-05-28 a wedged Browser object meant every recovery-page
load itself triggered another renderer termination, re-firing the
handler at ~108 events/sec for 22 minutes — 139,205 crashes, 884 MB
host log, and the user-visible "input is hard" symptom (host UI thread
CPU-pegged on synchronous log writes that starved the live renderer's
IPC).

Add a per-browser ring of crash timestamps to `AgentMuxHandler`,
keyed by `Browser::identifier()`. On each crash:

1. Prune entries outside `CRASH_BUDGET_WINDOW` (10 s).
2. Push the new timestamp.
3. If count exceeds `CRASH_BUDGET` (3), log `crash_loop_aborted` and
   load a terminal "give up" page (`crash_loop_terminal_page`)
   that has only a Quit button — no Reload, no `frame.load_url`
   target, so navigation cannot re-enter this handler.

The browser's history entry is dropped in `on_before_close` so the map
doesn't grow over a session.

Satisfies the prime directive of
`docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`:
"Bounded recovery — never an infinite restart loop."

Relates to #1117. Composes with #1119 (no silent fallback URL): if
both ship, the 2026-05-28 incident becomes a single new-window
failure with a clear error page — never a loop, never a degraded
host process.
