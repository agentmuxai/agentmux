# Analysis: reopening an existing "My Agents" pane triggers a multi-second frontend request storm

**Date:** 2026-08-23
**Trigger:** repo owner reopened a heavily-used agent ("AgentX," this session's
own agent) from My Agents and asked why loading an existing agent takes a
while, noting "docks and long-running tasks that show during load, then
disappear."
**Method:** this session's own agent pane reopening (block
`537ce695-737f-4a88-b354-7fc67fc1119c`, channel `local-main-b28b7a-e5bfaf58`,
build `0.55.21+gf59bb43b7`) was used as a live, real trace — not a synthetic
repro. Evidence is direct log correlation (host `[fe]` console-bridge lines +
`agentmux-srv` log), not inference from behavior alone.
**Status:** root-caused with high confidence (precise counts + code paths
confirmed on both frontend and backend). **Fixed** — §6 option 1 (frontend-only
debounce, no backend change) shipped same-day in
[PR #2773](https://github.com/agentmuxai/agentmux/pull/2773) as
`frontend/app/view/agent/activity/debounced-refresh.ts`, wired into both
`dispatch-source.ts` and `subagent-source.ts`; see
`docs/specs/SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md` for the
shipped design. Verified directly (2026-08-24): both modules route every
`subagent:*`/`dispatch:updated` handler through the shared debounce, and the
full activity test suite (77 tests) passes, including dedicated burst-collapse
and sustained-burst/max-wait-ceiling coverage. §6 options 2-4 below remain
unimplemented follow-ups, not required for the storm itself. This document is
being left otherwise unedited (as the original incident analysis) — this
status line is the only change.

---

## TL;DR

Reopening a pane with a large historical corpus of subagent (Task-tool /
Workflow) activity replays that entire (capped) history as a burst of
`subagent:spawned`/`subagent:completed` WebSocket events. Two independent
frontend singletons (`dispatch-source.ts`, `subagent-source.ts` — the exact
modules backing the Activity Dock's "long-running tasks" rows) each react to
*every single event in the burst* by firing its own uncoalesced
`ListDispatches`/`ListActive` RPC round trip, with zero debouncing. In this
trace, a 155-event, ~2.2s backend burst produced roughly 300 overlapping
frontend RPC calls that took **~7.6 seconds** to drain, during which:

- round-trip latency for the SAME two endpoints grew monotonically from
  ~84ms to over **7.2 seconds**, the classic signature of a request queue
  whose depth is growing faster than it can drain;
- the browser's own Long Task detector logged a 291ms main-thread block;
- an ordinary `main_window_focus` IPC round trip — normally a few
  milliseconds — took **2.08 seconds**, because the main thread was busy
  processing the storm;
- two unhandled promise rejections (`Cannot read properties of null
  (reading 'getBoundingClientRect')`) and repeated `ResizeObserver loop
  completed with undelivered notifications` errors fired, consistent with
  layout thrashing from the same overload.

The Activity Dock's "shows then disappears" rows the user is describing are
exactly `dispatch-source.ts`/`subagent-source.ts`'s consumers repainting on
every one of those ~300 refresh responses as they trickle back in — the dock
isn't glitching on its own, it's faithfully rendering a genuinely churning
data source.

## 1. What was directly observed (this session's own reopen, as a live trace)

Full annotated timeline, host log (`[fe]` = frontend console, bridged) and
`agentmux-srv` log, both for the exact same reopen:

| Time (UTC) | Source | Event |
|---|---|---|
| 12:06:04.859 | host `[fe]` | `[agent] Launching agent definition AgentX (claude)` — reopen begins |
| 12:06:04.892 | host `[fe]` | `WaveObj updated block:537ce695-...` |
| 12:06:04.899 | srv | `backfilling session subagents on pane (re)open` |
| 12:06:04.934 – 12:06:07.171 | srv | **155× `subagent spawned`** log lines (2.24s span) — the capped cold-backfill replay |
| 12:06:07.183 | host `[fe]` | `[reactive] registered agent AgentX -> 537ce695-...` |
| 12:06:07.216 | host `[fe]` | first `subagent.ListDispatches` (19ms) / `ListActive` (21ms) — still healthy |
| 12:06:07.327 → 12:06:14.862 | host `[fe]` | **the storm**: `subagent.ListDispatches`/`ListActive` fire essentially back-to-back, hundreds of times; each call's own reported round-trip grows over the window: 84ms → ~500ms (12:06:07.6) → ~2000ms (12:06:09.4) → ~5000ms (12:06:12.6) → **7201ms** (12:06:14.7, the last one) |
| 12:06:08.479 | host `[fe]` | `[perf] long-task 291.0ms name=self` — a single main-thread task blocked the browser for 291ms |
| 12:06:14.859 | host `[fe]` | `block.GetControllerStatus` itself reports **7180ms** — an unrelated call caught behind the same queue |
| 12:06:14.942–14.944 | host `[fe]` | 2× `[unhandled-rejection] TypeError: Cannot read properties of null (reading 'getBoundingClientRect')` |
| 12:06:15.056 | host `[fe]` | `[perf] ipc main_window_focus 2076.8ms` — a normally-instant native IPC call, starved by the same overload |
| 12:06:15.827, 16.521, 18.581, 18.695 | host `[fe]` | 4× `[uncaught-error] Error: ResizeObserver loop completed with undelivered notifications` |
| ~12:06:18.7 | host `[fe]` | settles into ordinary turn-based (`wave-turn`) activity — the storm is over |

**Total elapsed from "Launching agent definition" to genuinely settled:
~14 seconds.** The backend's own replay burst took only ~2.2s of that; the
remaining ~12s is the frontend queue draining and recovering.

## 2. Root cause

### 2.1 The backend replays capped, but real, history on every reopen

`agentmux-srv/src/backend/subagent_watcher/scan.rs`'s `scan_subagents_dir`
runs on every pane (re)open with a pre-existing session id
(`server/reactive.rs`), walking every `agent-*.jsonl` under the session's
`subagents/` directory (plus every workflow run's member files) and calling
`process_jsonl_change(..., live: false)` for each. This is the exact
mechanism a prior incident
([retro-subagent-backfill-storm-oom-2026-07-17.md](../retro/retro-subagent-backfill-storm-oom-2026-07-17.md))
already found and partially fixed: that incident hit **1,000+** replayed
files across repeated srv crashes and OOM'd the whole app. **Fix A** from
that retro caps the replay to `BACKFILL_MAX_FILES` (200) most-recently-
modified files — which is why this trace shows 155 events, not 1,000+, and
why the process didn't crash. That fix's own doc comment is explicit about
its scope: *"this only bounds the push (broadcast) side of a cold
backfill"* — it bounds the SIZE of the burst, not what happens to it once it
reaches the frontend.

### 2.2 `process_jsonl_change`'s `live` flag exists but never reaches the wire

`process_jsonl_change` (`agentmux-srv/src/backend/subagent_watcher/jsonl.rs`)
already distinguishes backfill replay from a genuine live spawn via its own
`live: bool` parameter — but that flag is currently used **only** to gate
the eager-Haiku-naming side effect (`if live && self.naming_triggered...`).
The actual WebSocket broadcast a few lines later —
`self.event_bus.broadcast_event(&spawned_event)` — fires unconditionally,
and the event payload sent to the frontend (`agentId`, `slug`, `parentAgent`,
`parentBlockId`, `sessionId`, `model`, `dispatchId`) carries no `live`/
`replay` field at all. The frontend has no way to tell "this is 1 of 155
historical events replaying in a 2-second burst" from "this is a single,
genuinely new spawn" — every one of the 155 looks identical to a live event.

### 2.3 Two independent frontend singletons each refresh, uncoalesced, per event

`frontend/app/view/agent/activity/dispatch-source.ts` and
`frontend/app/view/agent/activity/subagent-source.ts` are the app-lifetime,
singleton data sources behind the Activity Dock's dispatch cards and
subagent rows respectively (one shared instance across every open pane, by
design — see each file's own header comment). Both independently subscribe
to the same `subagent:spawned`/`subagent:completed`/`subagent:abandoned`
event types, and **both call their own unconditional, undebounced
`refresh()` on every single event**:

```ts
// subagent-source.ts
waveEventSubscribe({ eventType: "subagent:spawned", handler: () => void refresh() });
waveEventSubscribe({ eventType: "subagent:completed", handler: () => void refresh() });
waveEventSubscribe({ eventType: "subagent:abandoned", handler: () => void refresh() });

// dispatch-source.ts — same three, plus subagent:named and dispatch:updated
```

`refresh()` in both files is a bare `await callBackendService(...)` with no
in-flight guard, no debounce, and no coalescing — confirmed by reading
`callBackendService` itself (`frontend/app/store/wos.ts`): it is a plain
`fetch()` wrapper with no request de-duplication of any kind. A burst of N
backfill events therefore fires up to **2N** independent, fully-overlapping
HTTP round trips (N `ListActive` calls from `subagent-source.ts`, N
`ListDispatches` calls from `dispatch-source.ts`) into the same two backend
endpoints, all in flight simultaneously, none aware of the others. This is
the direct mechanical cause of the growing-latency queue observed in §1 —
each new request queues behind however many are already in flight, so
round-trip time grows roughly with elapsed time, exactly as measured (84ms
→ 7201ms across the ~7.5s window).

`dispatch-source.ts` does have a debounce/coalescing mechanism —
`scheduleQuietWindowRefresh` — but it solves a different, unrelated problem
(scheduling exactly one *delayed* follow-up refresh for a dispatch's lazy
running→completed transition). It only ever schedules a refresh that isn't
already pending; it does nothing to coalesce the *immediate* refreshes each
event handler fires on arrival.

## 3. What this is NOT

- **Not the Activity Dock shell-row flash bug**
  ([PR #2770](https://github.com/agentmuxai/agentmux/pull/2770),
  [retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md](../retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md)),
  even though the user's own description ("docks... show and then
  disappear") echoes that bug's report almost verbatim. That bug is about
  **shell** nodes specifically (`useShellNodeStream.ts`) momentarily
  rendering `status: "running"` for an already-long-dead shell before a
  correction lands — a display-correctness bug with a near-instant fix
  window, already root-caused and merged. This report is about **subagent**
  dispatch/active rows (`dispatch-source.ts`/`subagent-source.ts`) and is a
  **performance/request-volume** problem, not a display-correctness one —
  a different symptom (rows appearing and settling as data churns) with a
  superficially similar user description, but an unrelated mechanism and
  an unrelated fix. Note: the build this trace was captured on (`0.55.21+gf59bb43b7`,
  built before PR #2770 merged) does NOT yet include that fix either, so
  the shell-flash bug is *also* still live in this exact instance — but it
  is not what produced the ~14-second stall analyzed here.
- **Not a new OOM/crash risk.** The July 17 incident this traces back to was
  a crash; this is a multi-second UI stall. Fix A from that retro is working
  as designed (155 ≤ 200 cap; no crash, no restart-loop).
- **Not caused by any of this session's own recent frontend PRs** (#2761,
  #2768 — the pane-mount reveal-gate/cross-fade work). Those touch
  `agent-view.tsx`, `block.tsx`, `PaneTabStrip.tsx`, and related `.scss` —
  none of which are in the call path identified here. `dispatch-source.ts`/
  `subagent-source.ts` are untouched by either PR.
- **Not specific to this one agent's pane in principle** — the mechanism
  (backfill-on-reopen → uncoalesced per-event refresh) applies to any pane
  reopened with a nontrivial subagent/dispatch history. It's simply most
  visible on a heavily-used, long-running agent (this session's own AgentX
  pane has accumulated a large history from extensive Task-tool/Workflow
  usage across a long session) — which is exactly the "My Agents" reopen
  case the user was exercising.

## 4. Why "docks and long-running tasks... show then disappear"

The Activity Dock's subagent/dispatch rows are rendered directly from
`allSubagentsAtom`/`allDispatchesAtom` — the two signals `subagent-source.ts`/
`dispatch-source.ts` update on every `refresh()` resolution. During the
~7.6s storm, those signals update roughly 300 times as each queued response
trickles back (not simultaneously — they resolve in whatever order the
queue drains them, which is not necessarily arrival order), so the dock
genuinely re-renders with a rapidly, non-monotonically changing snapshot of
"currently active/dispatched" subagents until the last response lands and
the true final state settles. Rows that appear mid-storm and vanish once a
later, more-complete response supersedes them are not an animation glitch —
they are literally different intermediate answers to "what's active right
now," arriving out of order.

## 5. Quantifying the amplification

| Stage | Duration | Notes |
|---|---|---|
| Backend backfill replay | ~2.24s | 155 `subagent spawned` broadcasts, bounded by the existing 200-file cap |
| Frontend storm (drain) | ~7.6s | Up to ~310 overlapping RPC calls (155 × 2 sources), growing queue depth |
| Recovery tail (jank/errors) | ~4s | Long Task, starved `main_window_focus`, ResizeObserver loop errors, null-ref rejections |
| **Total, launch to settled** | **~14s** | vs. ~2.2s of actual new information from the backend |

The frontend spends roughly **6x longer processing the storm than the
backend spent producing it** — the amplification is entirely on the
consumption side, not the backfill-generation side.

## 6. Fix directions (not implemented — for discussion)

None of these have been built or verified; they're starting points.

1. **Coalesce bursts on the frontend (smallest, most targeted change).**
   Both `dispatch-source.ts` and `subagent-source.ts` could debounce their
   event-triggered `refresh()` calls — e.g. a short (50-100ms) trailing-edge
   debounce shared per module, so N events arriving within one burst collapse
   into a single `refresh()` call once the burst goes quiet, instead of N
   independent ones. This alone would cut the ~300 calls in this trace down
   to 2 (one per module). Doesn't require any backend change.
2. **Thread the existing `live` flag through to the wire.** Since
   `process_jsonl_change` already knows `live: false` for every backfill-
   replay event, adding a `"live": bool` field to the broadcast payload
   would let the frontend distinguish "historical replay" from "genuinely
   new" without any heuristic — a backfill-tagged event could always debounce
   hard (or even skip triggering `refresh()` per-event entirely, doing a
   single fetch once the pane is done backfilling), while a live event keeps
   today's immediate-refresh behavior for real-time responsiveness.
3. **In-flight de-duplication in `callBackendService` (broadest, most
   invasive).** A generic "if an identical in-flight call to this
   service+method exists, return that promise instead of issuing a new
   request" layer would help this specific storm and any future one shaped
   like it, but changes shared infrastructure every RPC call goes through —
   higher review/regression surface than options 1-2 for what is currently
   a single, localized hot spot.
4. **Cap harder or paginate the backfill itself** (revisit
   `BACKFILL_MAX_FILES`) — lower-leverage than 1-2, since even a smaller cap
   still produces an uncoalesced 1:1 refresh-per-event storm at a smaller
   scale; doesn't fix the mechanism, just shrinks the symptom.

**Recommendation for a first pass, if picked up:** option 1 is the smallest,
lowest-risk change that directly addresses the measured amplification
(§5's 6x factor), independently shippable without any backend change, and
directly testable (mock `waveEventSubscribe` firing N events synchronously,
assert `refresh`'s underlying RPC call fires once, not N times — the same
testing pattern already established in this codebase for
`useShellNodeStream.test.ts` and `tab-reveal.test.ts`). Option 2 is a good
complementary follow-up since it fixes the same problem more precisely (no
timing-based heuristic), but touches both backend and frontend and a wire
payload shape, so it's a larger, separate piece of work.

## 7. Open questions / not investigated here

- Whether `block.GetControllerStatus`, `object.UpdateObjectMeta`, and other
  `[fe] [service]` calls seen elsewhere in this same window are independently
  contributing any load, or are purely victims of the same queue (this
  report assumes the latter, based on `block.GetControllerStatus`'s own
  7180ms figure appearing at the same moment the ListDispatches/ListActive
  queue was at its deepest — but this wasn't isolated by, e.g., temporarily
  stubbing one call path and re-measuring).
- The two `getBoundingClientRect`-on-null unhandled rejections and the
  `ResizeObserver loop completed` errors are noted as corroborating evidence
  of main-thread overload, not separately root-caused — they may be a
  distinct, pre-existing bug (an unguarded ref read somewhere in the
  document/dock rendering path) that only *manifests* under this storm's
  timing pressure. Worth a dedicated look if they recur outside this
  scenario.
- Whether `BACKFILL_MAX_FILES` (200) is still an appropriate cap now that
  the *consumption*-side cost (this report's finding) is understood — the
  July 17 fix was tuned against the OOM-crash risk alone, not against
  frontend-storm duration.
