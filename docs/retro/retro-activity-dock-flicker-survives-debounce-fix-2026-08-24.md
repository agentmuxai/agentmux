# Retro: Activity Dock rows still show-then-disappear on pane reopen after the debounce fix

**Date:** 2026-08-24
**Reported by:** repo owner, on `0.55.23` — "the docked items still show, then
disappear... it should be the pulsating brain until everything is ready."
**Status:** root-caused; **no fix implemented** — this is an analysis writeup
per the request ("figure out why... write retro to file"). See §5 for
concrete, unimplemented fix directions.

---

## 1. Why this isn't a regression, and isn't the same bug already fixed

`0.55.23` already contains both recent fixes in this exact area:

- **PR #2773** (`e12f24b30`) — the debounce fix for
  `docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`'s
  request-storm (~300 overlapping RPC calls, 7.6s stall, growing latency,
  browser jank). Confirmed still in place and working: both
  `dispatch-source.ts` and `subagent-source.ts` route every subagent/dispatch
  event through `createDebouncedRefresh` (`frontend/app/view/agent/activity/debounced-refresh.ts`,
  trailing-edge 100ms + max-wait ceiling 1000ms).
- **PR #2777** — fixed the BrainSpinner overlay getting stuck **visible**
  forever (a different bug: the spinner never going away, not the dock
  flickering).

Neither PR claimed to fix dock-row flicker, and neither did. The storm's
*performance* symptom (multi-second stall, browser jank, growing RPC
latency) is genuinely gone. The *visual* symptom the repo owner is now
describing — rows appearing then disappearing during pane reopen — is a
narrower, still-live piece of the same original report that was explicitly
flagged as a separate, unaddressed layer at the time:

> §4 of the original report: *"Rows that appear mid-storm and vanish once a
> later, more complete response supersedes them are not an animation
> glitch — they are literally different intermediate answers to 'what's
> active right now,' arriving out of order."*

The debounce fix reduces how *often* the dock repaints during a reopen; it
was never going to make those repaints stop reflecting genuinely different,
transient backend states. That gap is what's still visible today.

## 2. Root cause, precisely

Two independent, compounding gaps:

### 2.1 The debounce coalesces request *volume*, not visual *settlement*

`createDebouncedRefresh(fn, waitMs, maxWaitMs)` (`debounced-refresh.ts`) is a
trailing-edge debounce with a hard ceiling — by design, it still calls `fn`
at least once every `maxWaitMs` (1000ms) if events keep arriving faster than
`waitMs` (100ms) apart. The original report measured the backend's own
cold-backfill replay (`scan_subagents_dir`, capped at `BACKFILL_MAX_FILES` =
200) taking **~2.24 seconds** to emit 155 `subagent:spawned` broadcasts —
i.e., events arriving roughly every ~14ms on average, well under the 100ms
window, so the trailing-edge timer keeps getting reset and essentially never
fires *during* the burst. The 1000ms ceiling is what actually fires instead:
over a ~2.2s continuous burst, that's **roughly 2-3 forced refreshes**
(≈1000ms, ≈2000ms, plus one final trailing-edge refresh ~100ms after the
burst goes quiet), each one a real, network-round-tripped `ListActive`/
`ListDispatches` call reflecting **whatever the backend's true state was at
that instant** — not a stale cache, not a glitch. Since subagents actually
do transition (active → completed / reconciled → abandoned, see
`reconcile_stale_subagents`) *while* the backfill is still streaming, those
2-3 snapshots are genuinely different from each other and from the final
settled state.

`ActivityDock.tsx:250`'s `<For each={renderedIds()}>` is a keyed list —
`mergeSubagentsPreservingIdentity` (`swarm-model.ts:458-467`) only preserves
**object identity** for items that appear unchanged across two snapshots
(a Solid reactivity optimization, so unaffected rows don't unnecessarily
re-render); it does nothing to keep an item in the rendered list that the
newer snapshot no longer contains. A subagent present in refresh #1 but
absent from refresh #2 (because it got reconciled to `abandoned` in the
interim, or simply hadn't been scanned into the backend's in-memory state
yet at the time of refresh #1) is a real DOM mount followed by a real DOM
unmount — literally "shows, then disappears," exactly as reported. **The
debounce fix was never designed to prevent this** — coalescing a storm of
requests into 2-3 requests still leaves 2-3 real, distinct, momentarily-true
answers to render.

### 2.2 The BrainSpinner ready-gate has nothing to do with this data source at all

`block.tsx:301`:

```ts
const ready = createMemo(() => !loading() && !isBlank(props.nodeModel.blockId) && blockData() != null && viewModel() != null);
```

`ready()` — the signal that shows/hides the BrainSpinner overlay
(`docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md`)
— depends **only** on this block's own `blockData`/`viewModel` resolving
from the object store. That's a fast, largely-local resolution (an
already-open pane's data is typically warm within the object store almost
immediately on mount) with **zero dependency** on `allSubagentsAtom`/
`allDispatchesAtom` (the Activity Dock's data, sourced from the
`dispatch-source.ts`/`subagent-source.ts` singletons, which are shared
across every pane in the app and have no per-block "ready" concept at all).

Concretely, per the original report's own directly-measured timeline: the
pane's `WaveObj updated` (an indicator `blockData` has landed) fired at
`12:06:04.892`, while the backend's subagent backfill didn't even *finish*
broadcasting until `12:06:07.171` and the frontend didn't fully settle until
`~12:06:18.7`. `ready()` — and therefore the BrainSpinner — almost
certainly flips to hidden **many seconds before** the Activity Dock's data
has converged. The spinner the repo owner wants to see "until everything is
ready" isn't gated on the dock being ready at all; it never was designed to
be. It covers "is this pane's own view usable," not "has every cross-pane
singleton data source this pane happens to render also settled."

### 2.3 The actual missing primitive: no signal exists for "backfill in progress" at all

The original report's §6 listed four fix directions. **Only option 1
(frontend debounce) shipped, in PR #2773.** Option 2 — *"thread the existing
`live` flag through to the wire"* — was never implemented. Confirmed
directly in current code: `process_jsonl_change`
(`agentmux-srv/src/backend/subagent_watcher/jsonl.rs:35`) still takes a
`live: bool` parameter, but it's used **only** to gate the eager-Haiku-naming
side effect (line 198) — the `subagent:spawned` broadcast payload itself
(lines 212-231) carries `agentId`/`slug`/`parentAgent`/`parentBlockId`/
`sessionId`/`model`/`dispatchId` and **nothing indicating whether this event
is a historical replay or a genuinely new spawn**. There is also no separate
"backfill finished" event anywhere in `scan_subagents_dir`
(`agentmux-srv/src/backend/subagent_watcher/scan.rs:374`) or its caller
(`scan.rs:68-70`) — the backend never tells the frontend "the cold replay
for this pane is done, you can trust what you have now."

**This is the actual missing piece for what the repo owner is asking for.**
Without it, the frontend has no principled way to distinguish "still
converging, don't render this yet" from "this is the real, final state" —
it can only ever guess via timing (a longer debounce window), which trades
flicker for added latency rather than eliminating it.

## 3. Why this matters more on this exact machine/agent

Nothing here is specific to one pane, but the *severity* scales directly
with subagent-history size: a lightly-used agent's reopen has few or zero
backfill events, so there's nothing to flicker between. The repo owner's own
long-running agents (the same class of pane the original report used as its
live trace, and the same class this very session's own agent has
accumulated — hundreds of subagent dispatches over an extended session) are
exactly the case where 150+ backend events compress into 2-3 real,
visibly-different frontend snapshots. A fresh or lightly-used agent would
likely never notice this at all, which is consistent with why this wasn't
caught by the original PR #2773 review (its own test coverage, correctly,
verifies debounce *coalescing behavior* with synthetic event bursts — it
doesn't and can't assert anything about the *content* of intermediate
snapshots, since that's a property of real, changing backend state, not of
the debounce mechanism itself).

## 4. What this is NOT

- **Not a reopening of the original request-storm bug.** Confirmed directly:
  both `dispatch-source.ts` and `subagent-source.ts` still route every event
  through the shared debounce; the full activity-dock test suite (77 tests)
  passes. The multi-second stall / growing-latency / browser-jank symptoms
  from the original report are not what's being described now.
- **Not the BrainSpinner-stuck-visible bug (PR #2777).** That bug was the
  spinner never disappearing; this is the opposite failure mode in the same
  neighborhood — the spinner disappearing correctly, just too early relative
  to a completely different data source it was never wired to.
- **Not a bug in `mergeSubagentsPreservingIdentity`.** It does exactly what
  its name says (preserve identity for unchanged items) and isn't
  responsible for deciding which items belong in the list at all — that's
  purely a function of what each backend `ListActive`/`ListDispatches`
  response contains at the moment it's fetched.

## 5. Fix directions (not implemented here)

1. **Thread a `live`/backfill-progress signal to the wire (closes §2.3,
   the real gap).** Add a field to the `subagent:spawned`/`subagent:completed`
   broadcast payload (or a new, separate `subagent:backfill_done` event
   fired once per pane once `scan_subagents_dir` finishes) so the frontend
   can tell "still replaying history" from "this is live/settled." This is
   `docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`
   §6's option 2, never implemented — and it's the only direction of the
   four that actually gives the frontend the information it's currently
   missing, rather than just tuning timing.
2. **Gate a NEW "backfilling" signal into the Activity Dock's own rendering,
   not `block.tsx`'s `ready()`.** Once (1) exists, the dock (or the
   whole-pane BrainSpinner, per the repo owner's stated preference) could
   suppress rendering `allSubagentsAtom`/`allDispatchesAtom` changes for a
   given pane until that pane's backfill is marked done, then reveal the
   single, final, correct state directly — zero intermediate flicker by
   construction, rather than by reducing its frequency. Extending
   `block.tsx`'s existing `ready()` memo to also depend on this new signal
   would reuse the BrainSpinner overlay already built for exactly this kind
   of "not ready yet" gating, matching the repo owner's own expectation
   ("it should be the pulsing brain until everything is ready") directly —
   though note `allSubagentsAtom`/`allDispatchesAtom` are app-wide
   singletons, not per-block, so "ready" would need to become per-pane-aware
   of a per-parent-block backfill flag, not a single global boolean.
3. **Alternative, smaller-scoped mitigation if (1)/(2) are too large:**
   lengthen the debounce's `maxWaitMs` specifically for a pane's *first*
   burst after reopen (vs. steady-state live events), trading a longer
   single delay before the dock ever shows anything for fewer/no
   intermediate flickers — strictly a mitigation (probabilistically reduces
   flicker for most reopens) not a fix (a sufficiently large/slow backfill
   could still flicker at the ceiling), and adds a "first burst vs.
   steady-state" distinction the debounce helper doesn't currently have.

**Recommendation if picked up:** (1) is the correct fix — it's the one
missing piece of information, matches the original report's own
recommendation for a "complementary follow-up... fixes the same problem
more precisely (no timing-based heuristic)," and is what (2) needs to exist
at all. (3) alone would not satisfy the repo owner's explicit ask ("it
should be the pulsing brain until everything is ready," not "flicker less
often") and shouldn't be presented as a full fix if implemented alone.

## 6. Evidence / sources

- `frontend/app/view/agent/activity/debounced-refresh.ts` — the shipped
  debounce primitive (100ms/1000ms), confirmed unchanged since PR #2773.
- `frontend/app/view/agent/activity/subagent-source.ts`,
  `dispatch-source.ts` — confirmed both still wire every event through
  `createDebouncedRefresh` with identical (100, 1000) parameters.
- `frontend/app/view/swarm/swarm-model.ts:458-467` —
  `mergeSubagentsPreservingIdentity`, confirmed to only preserve reference
  identity for unchanged items, not list membership.
- `frontend/app/view/agent/components/ActivityDock.tsx:250` — the keyed
  `<For each={renderedIds()}>` render that turns a changed snapshot into
  real mount/unmount.
- `frontend/app/block/block.tsx:301` — `ready()`'s exact definition,
  confirmed to have no dependency on Activity Dock data sources.
- `agentmux-srv/src/backend/subagent_watcher/jsonl.rs:35,198,212-231` —
  `process_jsonl_change`'s `live` parameter, confirmed still not present in
  the broadcast payload.
- `agentmux-srv/src/backend/subagent_watcher/scan.rs:68-70,374` —
  `scan_subagents_dir`, confirmed no "backfill done" event exists anywhere
  in this path.
- `docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md` —
  the original incident this retro follows up on, including the directly-
  measured timeline (§1) this retro's §2.2 timing argument is built from.
- `docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md`
  — the unrelated but adjacent BrainSpinner bug, confirmed distinct from
  this one in §4 above.
- `docs/specs/SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md` — the
  shipped debounce design doc.
