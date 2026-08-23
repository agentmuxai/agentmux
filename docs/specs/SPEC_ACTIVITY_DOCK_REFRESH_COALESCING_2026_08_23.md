# Activity Dock: coalesce event-triggered refreshes on pane reopen

**Status:** implemented (this PR)
**Owner:** AgentX
**Date:** 2026-08-23
**Scope:** `frontend/app/view/agent/activity/dispatch-source.ts`,
`frontend/app/view/agent/activity/subagent-source.ts`, a new shared
`frontend/app/view/agent/activity/debounced-refresh.ts`.
**Related:** `docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`
(the analysis this spec implements — read that first for the full
measurement/evidence; this doc covers only the fix design),
`docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md` (the prior,
backend-side fix this one complements — bounds the burst *size*, not what
the frontend does with it), `docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md`
(a different, already-fixed Activity Dock bug — display-correctness, not
performance; unrelated mechanism).

---

## 1. Problem (see the REPORT for full measurement)

Reopening a pane with a nontrivial subagent/dispatch history replays that
history as a burst of `subagent:spawned`/`subagent:completed`/
`subagent:abandoned`/`dispatch:updated` WebSocket events (backend-side
bounded to `BACKFILL_MAX_FILES` = 200 events per the July 17 fix). Two
independent, app-lifetime singleton modules behind the Activity Dock —
`dispatch-source.ts` (dispatch cards) and `subagent-source.ts` (subagent
rows) — each fire their own **uncoalesced** `ListDispatches`/`ListActive`
RPC call on every single event in that burst, with no debounce and no
in-flight de-duplication anywhere in the call stack (`callBackendService`
is a bare `fetch()` wrapper). A measured live trace: 155 backfill events →
~300 overlapping RPC calls → round-trip latency for the same two endpoints
grew from 84ms to 7.2s as the queue backed up, ~14 seconds total from pane
reopen to settled, with knock-on main-thread jank (a Long Task, a starved
`main_window_focus` IPC call, ResizeObserver loop errors).

## 2. Design

### 2.1 Decision: trailing-edge debounce with a max-wait ceiling, frontend-only

Of the four directions the REPORT sketched, this implements **option 1**
(smallest, most targeted, no backend change): debounce each module's
event-triggered `refresh()` so a dense burst of events collapses into one
actual RPC call once the burst goes quiet, instead of one per event.

- **Trailing-edge, not leading-edge or throttle**: the goal is "wait until
  the burst is over, then fetch the final state once" — a leading-edge call
  would fire on the FIRST event and then need updating again once later
  events land anyway (no better than today); a fixed-interval throttle
  would still fire many times across a 2+ second burst.
- **Max-wait ceiling**: without one, a continuous stream of events spaced
  closer together than the debounce window would defer the underlying call
  indefinitely (each new event resets the trailing timer before it fires).
  A ceiling forces a refresh periodically even under sustained load, at the
  cost of occasionally firing more than the theoretical minimum — a safe
  trade.
- **Chosen values: 100ms trailing wait, 1000ms max wait.** 100ms is well
  under the threshold of a perceptible delay for a *live* spawn/completion
  event (the debounce applies uniformly to both backfill-replay and live
  events in this pass — see §2.2 for why), while being long enough that the
  measured backfill burst's typical inter-event spacing (155 events over
  2.24s, ~14ms average) collapses into one call. 1000ms as a ceiling means
  even a pathologically dense, sustained stream refreshes at least once per
  second — frequent enough to still feel responsive, far short of today's
  effectively-uncapped-until-the-queue-drains behavior.
- **Frontend-only, no backend change**: `process_jsonl_change`'s existing
  `live: bool` parameter (already computed server-side, currently used only
  to gate the eager-naming side effect) is a good foundation for a more
  precise future fix — threading it through to the wire and using it to
  batch backfill-tagged events differently from live ones (the REPORT's
  option 2) — but that's a larger, separate change touching a wire payload
  shape on both sides. Deferred; not needed to fix the measured storm, since
  a uniform debounce already collapses both cases correctly.

### 2.2 Why apply the debounce uniformly to backfill AND live events

The frontend currently has no signal distinguishing a backfill-replay event
from a live one (§2.1's deferred option 2 would add one). Given that, the
only implementable-today choice is a single debounce policy applied to
every event regardless of origin. This is a deliberate, low-cost trade: a
genuinely live subagent spawn during active use is delayed by at most
~100ms before the dock reflects it (imperceptible), in exchange for
collapsing a 200-event backfill burst into a single call. If option 2 is
built later, live events could bypass the debounce entirely for maximal
real-time responsiveness while backfill events keep collapsing — not
needed for this pass.

### 2.3 Shared helper, not two copies

`dispatch-source.ts` and `subagent-source.ts` need byte-for-byte identical
debounce behavior (same constants, same trailing+ceiling semantics) — a
genuine, not premature, case for extracting `debounced-refresh.ts`: two
real call sites needing the same tuned behavior, where drift between them
(one file's debounce window silently diverging from the other's during a
future edit) would be a real, easy-to-introduce bug. The extracted function
takes the underlying callback and both timings as parameters — no other
app-specific coupling — so it's independently unit-testable without
mocking either source module.

### 2.4 What does NOT change

- The module-load-time initial `void refresh()` call in both files stays
  immediate, undebounced — the first paint of the dock should reflect
  real data as soon as possible, not wait out a debounce window with
  nothing to coalesce against yet.
- `dispatch-source.ts`'s existing `scheduleQuietWindowRefresh` (the
  lazy running→completed quiet-window follow-up) is unrelated and
  untouched — it already only ever schedules one pending timer, solving a
  different problem (a single deferred refresh at a known future deadline,
  not coalescing a burst of already-arrived events).
- `subagent:named`'s handler in `subagent-source.ts` is untouched — it
  patches `allSubagents` locally from the event payload directly and never
  calls `refresh()`, so it was never part of the storm.

## 3. Implementation

`debounced-refresh.ts` exports `createDebouncedRefresh(fn, waitMs,
maxWaitMs)`, returning a `trigger()` function. Each `trigger()` call:
resets the trailing timer to fire `fn` after `waitMs` of quiet; arms a
separate max-wait timer (only if one isn't already pending) that
force-fires `fn` after `maxWaitMs` regardless of continued triggers. Either
timer firing clears both (so a max-wait fire doesn't leave a stale trailing
timer that fires `fn` a second time shortly after).

Both `dispatch-source.ts` and `subagent-source.ts` replace their event
handlers' direct `() => void refresh()` with `() => scheduleRefresh()`,
where `scheduleRefresh = createDebouncedRefresh(() => void refresh(), 100, 1000)`
is constructed once at module scope (same singleton-per-module lifetime as
everything else in these files).

## 4. Testing

- New `debounced-refresh.test.ts`: fake timers: N rapid `trigger()` calls
  collapse to exactly 1 `fn` call after `waitMs`; a trailing call within the
  window resets the timer (still only 1 call, later); a call spaced past
  `maxWaitMs` since the first trigger fires early via the ceiling even
  under continuous triggering; independent instances don't interfere.
- `subagent-source.test.ts`: existing "also still refreshes on
  subagent:spawned and subagent:completed" test updated — two events fired
  back-to-back (microtask-only spacing) now correctly assert exactly ONE
  underlying `ListActive` call once fake timers advance past the debounce
  window, not two immediate ones (the old assertion encoded the pre-fix,
  uncoalesced behavior). A new test confirms the coalescing directly: many
  rapid `subagent:spawned` events collapse into one `ListActive` call.
- `dispatch-source.ts` gets an equivalent new describe block (previously had
  zero event-wiring test coverage at all — see `dispatch-source.test.ts`'s
  pre-existing scope, limited to the pure `msUntilNextQuietWindowRefresh`
  predicate).

## 5. Non-goals (this pass)

- Threading the backend's `live` flag through to the wire (REPORT §6
  option 2) — a larger, separate change.
- In-flight de-duplication inside `callBackendService` itself (REPORT §6
  option 3) — broader shared-infrastructure change, higher review surface,
  not needed once the two known call sites debounce.
- Revisiting `BACKFILL_MAX_FILES` (REPORT §6 option 4 / §7) — this fix
  makes the cap's frontend cost roughly constant regardless of its value,
  so there's less pressure to tune it further as a result of this work.
