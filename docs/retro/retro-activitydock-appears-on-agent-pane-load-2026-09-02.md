# Retro: ActivityDock flashes stale subagent rows on agent pane load (again)

**Date:** 2026-09-02
**Reported by:** repo owner, live session — "we had a lot of work that made
sure the agent pane loaded smooth... but now when the agent pane loads, I
see the long-running tasks dock." Confirmed on follow-up: a flash lasting a
couple of milliseconds to a few seconds, self-corrects.
**Status:** Root cause identified with direct evidence (live trace + prior
project history). Not a regression of any single commit — it's the
previously-documented, explicitly-still-open structural gap in this feature
area surfacing again on a heavy agent (Lzop).

---

## TL;DR

This bug (or a close relative of it) has been reported and partially fixed
**four times before** across 2026-08-22 through 2026-08-27
(`retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md`,
`REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`,
`retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md`,
`REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md`). Each fix
closed one specific, measured mechanism. The last of those four reports
(2026-08-27) explicitly diagnosed **why the whole family keeps recurring**
and proposed the actual structural fix — a single pane-scoped "paint gate"
that nothing renders until every async data source has registered as ready.
That fix (§4 of the 08-27 report) was **never built** — marked open then,
still open now (no commit since `c58ce9f1` touches it). Today's flash on
Lzop is that same open gap, not new breakage.

## Live evidence

A `MutationObserver` installed via CDP against Lzop's actual running pane
(`local-main-b28b7a-8505b7b7`, block `f5c69569-…`) caught the flash directly
on a reopen the repo owner triggered:

```
t=9510130ms  ADDED    "■ a2e5b60979142ecb3 [0:00] ↳ 52 events
                       ■ magical-enchanting-diffie [0:00] ↳ 7 events
                       ■ magical-enchanting-diffie [0:00] ↳ 28 events
                       ▸ 16 more (16 subagents)"
t=9513507ms  REMOVED
```

~20 subagents render as live dock rows, then all disappear ~3.4s later.
This is **not** the orphaned-Bash-tool-call theory from the first pass of
this retro (that data is real and does sit in Lzop's history, but the
settled document state correctly clamps it out — confirmed separately, see
"What this is NOT" below). It's the subagent source,
`frontend/app/view/agent/activity/subagent-source.ts`'s `allSubagentsAtom`,
fed by `subagent.ListActive`.

## Why this exact shape, on this exact agent

Straight from `REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md`
§2-3, still accurate today:

1. On every pane reopen, the backend cold-replays the agent's historical
   `agent-*.jsonl` subagent files (`scan_subagents_dir`, capped at
   `BACKFILL_MAX_FILES = 200`) to reconstruct "what subagents does this
   agent know about." Each replayed file broadcasts `subagent:spawned`.
2. **PR #2837 (v0.55.28, shipped, still in place)** fixed the worst part of
   this: a replay whose parent turn has already ended is now born
   `Abandoned` instead of `Active`, removing the specific "assert 200 fake-
   active subagents, then retract them 314ms later" transient the 08-27
   report measured.
3. What was **never fixed** (08-27 report §4, marked open, unstarted): there
   is still no single, deterministic "this pane's data is fully settled,
   render now" barrier. What exists instead is four independent, timing-
   based approximations — `createDebouncedRefresh` (100ms/1000ms),
   `useSubagentBackfillGate` (250ms buffer + 20s safety net),
   `createBackfillAwareTrigger` (20s lost-event fallback), and
   `shellStatusCorrection` (no timeout at all, races the exit-chunk ring).
   None of them *guarantee* zero flicker — they reduce its probability for
   the burst shapes they were tuned against.
4. The report's own §3 called out explicitly which agents this hits hardest:
   *"a lightly-used agent's reopen has few or zero backfill events... The
   repo owner's own long-running agents... are exactly the case where 150+
   backend events compress into 2-3 real, visibly-different frontend
   snapshots."* Lzop — 72,260 lines of transcript, extensive subagent
   history (matching the "16-20 subagents" in the live capture) — is
   precisely that worst case. A lighter agent's pane would likely show
   nothing at all, which is consistent with why this wasn't caught again
   until now.

So: **the backend no longer lies about subagent status (fixed), but the
frontend still has no hard guarantee it won't sample and render mid-burst
(never fixed)** — for a heavy-enough agent, that residual timing gap is
still wide enough to paint and unpaint a real (if momentary) intermediate
state.

## What this is NOT

- **Not the orphaned-tool-call theory this retro originally proposed.**
  Lzop's transcript genuinely contains 5 permanently-`"running"` orphaned
  Bash `tool_use` calls (real debris from a "someone just killed the
  instance" incident on v0.49.13, weeks old) — but live CDP inspection of
  the settled pane confirmed `clampToSessionScope` correctly excludes them
  (`dockCount: 0`, correct "NEW SESSION STARTED" banner shown). That code
  path is fine. Left as a documented dead end so it isn't re-investigated.
- **Not a regression from PR #2932** (this session's own Armory reactive-
  updates work) or from PR #2930/#2933 (the subprocess/container turn and
  credential-mounting fixes merged just before it) — none of those touch
  `subagent_watcher/`, `subagent-source.ts`, or any of the four mechanisms
  in the table above. That line of investigation (the first pass of this
  retro) is superseded.
- **Not something PR #2837 failed to fix.** Its own scope (stop asserting
  false-`Active` state on replay) is intact and working — verified no
  commit has touched `jsonl.rs`/`scan.rs`'s relevant functions since.

## What would actually close this

Exactly what `REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md` §4
already specified and scoped, still unbuilt:

> **A participant-registered paint gate.** Each async source that can
> change first-paint content registers a token at mount
> (`"history-replay"`, `"subagent-backfill"`, `"shell-status"`,
> `"background-registry"`). The pane paints when the token set empties, or
> when one shared deadline expires. Adding a new async surface means
> registering a token, not discovering `ready()` and wiring in a fifth
> ad-hoc gate.

This is a real, moderately-sized frontend change (a new pane-scoped
readiness registry consumed by `block.tsx`'s `ready()`/BrainSpinner, plus
updating the ~4 existing mechanisms to register/deregister tokens instead
of independently guessing timing), not a quick patch. Given it's already
fully designed and scoped in an existing report, the right next step is
implementing §4 as specified there — not inventing a new approach.

## Sources

- `docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md` —
  the direct answer to "how did it come back": §4 was always still open.
- `docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md`,
  `docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`,
  `docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md`
  — the three earlier fixes in this family, all still intact, none of them
  claiming to fully close the flicker class.
- `frontend/app/view/agent/activity/subagent-source.ts`,
  `hooks/useSubagentBackfillGate.ts`, `activity/backfill-tracker.ts`,
  `activity/debounced-refresh.ts` — the four still-live, still-approximate
  mechanisms.
- `agentmux-srv/src/backend/subagent_watcher/{jsonl,scan}.rs` — confirmed
  PR #2837's fix (`c58ce9f1`) is the latest change, still in place.
- Live trace this session: CDP `MutationObserver` against
  `ws://127.0.0.1:60304/devtools/page/…` (Lzop's actual running pane,
  channel `local-main-b28b7a-8505b7b7`, block
  `f5c69569-5763-4319-a114-07864ecd06d9`), captured on a reopen the repo
  owner triggered directly.
