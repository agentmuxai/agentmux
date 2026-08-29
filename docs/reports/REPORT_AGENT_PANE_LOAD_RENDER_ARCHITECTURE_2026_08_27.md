# Report: agent pane load is still glitchy on 0.55.26 — why the fixes didn't deliver "brain, then a clean paint"

**Date:** 2026-08-27
**Trigger:** repo owner, on the released `0.55.26` build (the first build that
contains *all four* prior fixes in this family): *"I still see it .. when
loading you inside of 55.26, I saw the docks load then flicker away. we simply
want a pulsating brain logo loading, and then a clean transition to a fully
rendered conversation. instead we get a lot of glitches. perhaps the render
modules need a rethink."*
**Method:** a live trace of the exact reported load — AgentA's own pane
reopening at `19:47:33Z` today on `0.55.26`, block
`58734821-3bd8-42ca-9a5d-779148dd7c02`, channel `local-main-b28b7a-aeca8206` —
correlating the host `[fe]` console bridge against the `agentmux-srv` log, plus
a direct read of the current gate/transport code on `main` (`84cc072d4`). Not a
synthetic repro, and not doc archaeology.
**Status:** Analysis only. No code changed. §2 and §5 are established by direct
evidence; §3 and §4 are structural findings from the code; §6 lists what this
report deliberately does **not** claim.

---

## TL;DR

The 08-25 architecture analysis said the flicker was probably just a release
gap — that all the fixes were merged but untagged. That explanation is now
**dead**. `v0.55.26` shipped today and contains PR #2770, #2773, #2781 and
#2801 (verified by `git merge-base --is-ancestor`). The symptom survived.

The live trace shows the *performance* half of the problem is genuinely fixed
and the *visual* half is untouched, for a reason none of the four fixes
addressed:

**On every reopen, the backend fabricates ~200 subagents into its own
"currently active" set, broadcasts them as spawns, and then — 314ms later,
while the same load is still in flight — retracts all 200 as abandoned.** The
Activity Dock is not glitching. It is faithfully rendering a backend that
genuinely believes, for about half a second, that 200 long-dead subagents are
running right now. Every fix so far has attacked *when the frontend samples*
that state (debounce → gate → spinner gate). None changed the fact that **the
state itself is wrong during the sampling window.**

That is why the fixes keep almost working, and why each one needs a new
timeout: they are four independent attempts to avoid observing a transient lie,
instead of one change that stops telling it.

---

## 1. The live trace: what 0.55.26 actually fixed

The request storm is genuinely, measurably gone. Same pane, same agent, same
measurement as `REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md` §1:

| Metric | 0.55.21 (08-23 trace) | 0.55.26 (this trace) |
|---|---|---|
| `ListActive` + `ListDispatches` calls | ~310 | **7** |
| Worst RPC round-trip | 7201 ms | **56 ms** |
| Longest main-thread block | 291 ms | 406 ms |
| Unhandled rejections / ResizeObserver errors | 6 | **0** |
| Launch → settled | ~14 s | **~1.9 s** |

PRs #2773 and #2801 did what they claimed. Nobody should revert them, and the
remaining problem is not "the debounce didn't work."

But inside that 1.9 seconds, this happened (srv log, same window):

| Time (UTC) | Source | Event |
|---|---|---|
| `19:47:33.535` | `[fe]` | `Launching agent definition AgentA (claude)` |
| `19:47:33.592` | srv | `backfilling session subagents on pane (re)open` |
| `19:47:33.622` | srv | `scan_subagents_dir: capping cold-backfill replay to the most recent files` |
| `19:47:33.622` | srv | `publish_backfill_status(..., "started")` — the gate closes |
| `19:47:33.624` – `34.052` | srv | **200 × `subagent spawned`** (428 ms) — every one inserted as `Active` |
| `19:47:33.938` – `33.954` | srv | **200 × `subagent reconciled: active -> abandoned (parent turn ended)`** (16 ms) |
| `19:47:33.975` | `[fe]` | `[reactive] registered agent AgentA -> 58734821-…` |
| `19:47:34.070` / `34.079` | `[fe]` | `subagent.ListActive` 51 ms / `ListDispatches` 56 ms |
| `19:47:34.286` / `34.293` | `[fe]` | `ListActive` 13 ms / `ListDispatches` 14 ms |
| `19:47:34.344` / `34.353` | `[fe]` | `ListActive` 15 ms / `ListDispatches` 22 ms |
| `19:47:34.873` | `[fe]` | `long-task 406 ms name=self` |
| `19:47:35.351` | `[fe]` | last of five long-tasks (75/406/62/82/139 ms) |

Three distinct refresh rounds reached the dock inside a 280 ms window that
straddles the end of the backfill burst — and the backend's own answer to
"what is active right now" changed 400 times inside that same window. Three
different answers, rendered. That is the flicker, exactly as the owner
described it, on a build with every fix in place.

## 2. Root cause: the backfill asserts 200 false "active" subagents, then retracts them

This is the finding the previous four fixes never touched.

`scan_subagents_dir` (`agentmux-srv/src/backend/subagent_watcher/scan.rs`)
replays up to `BACKFILL_MAX_FILES` (200) historical `agent-*.jsonl` files
through `process_jsonl_change(..., live: false)`. Each replayed file produces a
`subagent:spawned` broadcast **and an entry in the watcher's live `sessions`
map with `status: SubAgentStatus::Active`** — because a spawn record, replayed,
looks exactly like a spawn.

Nothing in that path knows these subagents finished hours or days ago. The
correction comes from a *separate* pass, `reconcile_stale_subagents`
(`scan.rs:210-285`), which walks the same map, flips every `Active` entry owned
by this block to `Abandoned`, and calls `broadcast_subagents_abandoned`. In
this trace that pass fired at `33.938` — **314 ms after the first fabricated
spawn, and 98 ms before the last one.** For that window, `subagent.ListActive`
— the exact RPC the dock's rows are built from — returns up to 200 rows that
are, by the backend's own subsequent admission, not active.

So the sequence the user sees is not a rendering artifact at any layer:

1. Backend says: 200 subagents are active. (True as far as the replay knows.)
2. Dock renders 200 rows. (Correct, given the data.)
3. Backend says: actually all 200 are abandoned. (Also true.)
4. Dock removes 200 rows, playing its departure animation. (Correct again.)

**Every component behaved correctly. The composition is what's broken.** This
is the same root cause the 08-25 analysis named — the backend never
distinguishes replay from live — but one level deeper than that report framed
it. It is not only that the *event* lacks a `live` flag; it is that the
*server-side state model* has no representation for "a subagent that existed,
in the past, and is over." Replay and live share one mutable `Active` set, so
replaying history necessarily corrupts the present, and a second pass has to
un-corrupt it.

The fix direction that follows is not another gate. It is: **a replayed spawn
whose parent turn has already ended must never enter the `Active` set in the
first place.** `reconcile_stale_subagents` already computes exactly that
predicate — 16 ms after the fact, on data it could have had up front. Applying
it at insertion time makes both the abandon-broadcast storm and the transient
lie structurally impossible, and makes the dock correct with *no* frontend gate
at all.

## 3. Four independent approximations of one missing fact

Because that fact was never available, four separate mechanisms now exist to
guess at it. All four are in `0.55.26`, all four are live, and each has its own
timeout constant:

| Mechanism | File | Approximates | Its own timeout |
|---|---|---|---|
| `createDebouncedRefresh` (#2773) | `activity/debounced-refresh.ts` | "the burst is over" | 100 ms trailing / 1000 ms ceiling |
| `useSubagentBackfillGate` (#2781) | `hooks/useSubagentBackfillGate.ts` | "this block is done backfilling" | 250 ms buffer + 20 000 ms safety net |
| `createBackfillAwareTrigger` (#2801) | `activity/backfill-tracker.ts` | "any block is backfilling" | 20 000 ms lost-event fallback |
| `shellStatusCorrection` (#2770) | `hooks/useShellNodeStream.ts` | "this replayed shell already exited" | none — races the chunk ring |

Two of these parse the *same* `subagent:backfill_status` event into the *same*
two string states with *two separate, deliberately non-shared* parse functions
(`resolveBackfillStatus` and `parseBackfillStatusEvent`), each carrying a
comment explaining why it doesn't reuse the other. That is not an accident of
sloppiness — both comments are thoughtful. It is what happens when the real
signal is missing and each consumer has to invent a proxy.

The cost is concrete and already paid: `useSubagentBackfillGate.ts` took
**eight review rounds** (P0s in rounds 4 and 5, P1s in rounds 2 and 8) to get a
single boolean right, because "has this pane finished loading" was being
derived from a racing async read rather than known. PR #2770 took three rounds
and introduced a worse bug than it fixed on the way. The bug class is not
getting cheaper to fix; it is getting more expensive.

There is also a live transport asymmetry worth naming, since it bounds how
reliable any gate of this shape can ever be. `subagent:spawned` is sent via
`EventBus::broadcast_event` (`eventbus.rs:171`) — an unconditional push to
every connected client, no scope, **no persist ring**. `subagent:backfill_status`
is sent via `Broker::publish` (`wps.rs:389`) — route-matched, scope-filtered,
`persist: 2`. Both land on `Lane::Priority`, so a subscriber holding both
subscriptions at publish time does see them in order (verified:
`waveEventSubscribe` with no `scope` sets `allscopes: true`, so the tracker's
unscoped subscription does match block-scoped events — an earlier hypothesis
that the gate never receives its event at all was checked and is **wrong**).
But the two channels have different delivery semantics for anything that
subscribes late or reconnects: the gate signal replays from its ring, the
events it gates do not. A gate and the thing it gates should not travel on
transports with different durability guarantees.

## 4. Why "brain, then one clean paint" can't be delivered by patching this

The owner's ask is a **single global readiness barrier**: nothing paints until
everything is ready, then one transition. What exists instead is `ready()` — a
single boolean in `block.tsx` that each async source must be individually
taught to hold down.

There are **26 modules** under `frontend/app/view/agent/` that independently
call `waveEventSubscribe` (excluding tests). Each maintains its own async
settle path. `ready()` currently accounts for three of them. Every future
surface — a new dock row type, a new status strip, a new inline panel — is a
new opportunity to paint before the pane is ready, and the only defence is
remembering to wire another gate into the same boolean. That is the structural
reason this keeps regressing after each fix: **the barrier doesn't scale with
the number of things it has to bar.**

Supporting evidence that this module is past its structural limit:
`agent-view.tsx` is **2,504 lines** against a documented 300-line budget
(8.3×, per `SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md`, unchanged since the
08-25 report measured it); `activity/` is 2,760 lines across 20 files, of which
two (`dispatch-source.ts`, `subagent-source.ts`) are self-described
hand-kept-in-sync copies of each other.

**Recommended shape — a participant-registered paint gate.** Replace the ad-hoc
booleans with one pane-scoped readiness registry:

- Each async source that can change first-paint content registers a token at
  mount: `"history-replay"`, `"subagent-backfill"`, `"shell-status"`,
  `"background-registry"`.
- The pane paints when the token set empties, or when **one** shared deadline
  expires — one timeout policy, one place to reason about failure, instead of
  the four independent constants in §3.
- Adding a new async surface means registering a token, not discovering
  `ready()` and wiring a fifth gate into it.
- The BrainSpinner is the natural rendering of "tokens outstanding," which is
  exactly the UX asked for.

This is deliberately **not** a rewrite of the backend subagent/shell/dispatch
subsystems — the 08-25 report's judgement that the domain split is legitimate
still holds. It is a frontend readiness primitive plus the §2 backend fix. On
its own the paint gate would only *hide* the transient lie; §2 is what removes
it. Both are needed, and §2 is the higher-value one — it makes three of the
four mechanisms in §3 deletable.

## 5. A separate, larger finding: `output.idx` is fully rebuilt every 30 seconds, forever

Found while tracing the above, and worth its own work item. It is not the
flicker's cause, but it is a permanent background jank generator on exactly the
"old, heavily-used agent" panes this whole report is about.

From the same srv log, one block (`agent:d76da857-…:current` — AgentA's global
transcript zone, **759 MB / 823,222 lines**):

- **37 full rebuilds** between `19:48:12` and `20:07:36` — a 19m 24s (1 164 s)
  window, chosen as a closed interval only because the log kept growing while
  this was measured; the cadence has not stopped.
- Durations 2 593 ms – 8 679 ms, mean **3 168 ms**, total **117.2 s**.
- That is a **10.1% duty cycle** of full-file streaming rescan, sustained,
  indefinitely, for one agent.
- Between two consecutive rebuilds the file grew by **13 662 bytes**. All
  759 MB is re-scanned from byte zero to index that 13 KB.

The chain, confirmed end to end:

1. `useSnapshotPersistence.ts:133` — a 30 s dirty-flag interval, per open agent
   pane.
2. It calls `BlockfileLineCountCommand(block_id, "output")` **with
   `{ timeout: 3000 }`** (line 74) purely to obtain a `highWaterMark` for the
   snapshot.
3. srv's `blockfile:line_count` misses its O(1) `session:line_count` meta fast
   path for this block and falls through to the global-zone path.
4. `output.idx`'s 8-byte covered-size header no longer matches the file size —
   because a live agent is appending to it continuously — so it is never fresh.
5. `rebuild_output_idx` (`shell/indexing.rs:32`) does a full streaming rescan,
   **synchronously, with no `tokio::task::spawn_blocking`**, occupying a Tokio
   worker on the shared srv runtime for ~3 s.

Three things are wrong here, independently:

- **The caller has already given up.** The RPC times out at 3 000 ms; the mean
  rebuild is 3 168 ms. The majority of these scans produce a result nobody is
  still waiting for — and the 8 679 ms cold one definitely did. The work is
  then repeated 30 s later.
- **The index is rebuilt in full for an append.** This is exactly follow-up #3
  from `STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md` §7
  ("investigate incremental/append-only indexing"), still open. The current
  design's own doc comment argues a pure rebuild "can never desync" — true, and
  a reasonable call at the time, but it was never costed against a
  continuously-growing 759 MB file.
- **It still isn't offloaded.** §7.1 of that same status doc flagged the missing
  `spawn_blocking` as "a real, independently-worth-fixing gap" five days ago. It
  is unchanged on `main`.

The one thing that *did* land from that doc is its §7.1 recommendation to
instrument first: `"output.idx rebuild starting"` with a matching `duration_ms`
on every exit path now exists (PR #2724), and it is the only reason this
section can quote real numbers instead of a hypothesis. That was the right call
and it paid off immediately.

Cheapest correct fix, in order: (a) let the snapshot's `highWaterMark` come
from the O(1) meta or be omitted rather than forcing an index build — a
snapshot already soft-fails without it; (b) make the index append-only
(the header already records covered size, so an append is `scan from
covered_size`, not from 0); (c) `spawn_blocking` the rebuild regardless.

## 6. What this report does not establish

Stated explicitly, because the two prior reports in this family were both
corrected for over-attribution and the discipline is worth keeping:

- **Why three refresh rounds got through rather than one.** The gate logic in
  `createBackfillAwareTrigger` reads correctly, the event delivery path was
  verified to reach it, and the host process had been running for 12 hours so
  no module-load race applies. The trace shows three rounds landed anyway. This
  report does **not** claim the gate failed, nor that it worked and something
  else fired the extra rounds — the logs cannot distinguish
  "gate never engaged" from "gate engaged, opened at `done`, and two unrelated
  post-settle events fired." Resolving it needs a frontend-side log of the
  tracker's open/close transitions and per-refresh trigger attribution. It is
  also **not on the critical path**: §2 makes all three rounds return the same
  correct answer, at which point the count stops mattering.
- **Whether the five long-tasks (75/406/62/82/139 ms) come from the dock
  churn or from history replay and virtualizer measurement.** Both are active
  in that window. Not isolated.
- **Whether the `blockfile:line_count` fast-path miss in §5 is specific to
  global-zone (cross-channel) blocks** or applies to local blocks too. The
  trace only exercised one block.
- **No claim that any prior fix should be reverted.** §1's numbers show they
  work. The argument in §3 is that three of them become *unnecessary* after
  §2 — which is a reason to consolidate later, not to undo now.

## 7. Recommended sequence

1. **§2 — stop asserting false-active state on replay.** Apply
   `reconcile_stale_subagents`'s existing predicate at insert time in
   `process_jsonl_change` when `live: false`, so a replayed subagent whose
   parent turn has ended is born `Abandoned`. Highest value, backend-only,
   directly testable (assert `ListActive` never transiently exceeds its settled
   count across a backfill), and it retires most of §3.
2. **§5(a) — stop the snapshot poller forcing a 759 MB rescan every 30 s.**
   One-line-ish frontend change, immediate and large win on exactly the heavy
   panes in question.
3. **§4 — the participant-registered paint gate.** The actual "rethink," but
   scoped: one readiness primitive, not a rewrite. Do it after 1 so it is
   hiding nothing.
4. **§5(b,c) — append-only `output.idx` + `spawn_blocking`.** Independent
   track; fixes a latent whole-app-stall risk documented on 08-22 and still
   open.
5. Then delete what 1 and 3 made redundant in §3, rather than leaving four
   overlapping guards in place. Per the 08-25 report's Tier 3 note, this should
   be a tracked item — 7 of 8 prior "clean this up later" notes in this family
   were never picked up.

## 8. Sources

- Live trace: `~/.agentmux/channels/local-main-b28b7a-aeca8206/versions/0.55.26/logs/agentmux-host-v0.55.26.log.2026-08-27`
  and `.../agentmuxsrv-v0.55.26.log.2026-08-27`, window `19:47:30Z`–`20:07:40Z`.
- `agentmux-srv/src/backend/subagent_watcher/scan.rs` (`publish_backfill_status`,
  `scan_session_subagents`, `reconcile_stale_subagents:210-285`)
- `agentmux-srv/src/backend/subagent_watcher/jsonl.rs` (`process_jsonl_change`,
  the `live` flag, `spawned_event` broadcast at :212/:370)
- `agentmux-srv/src/backend/eventbus.rs:171` / `agentmux-srv/src/backend/wps.rs:389`
  (the two transports)
- `agentmux-srv/src/backend/blockcontroller/shell/indexing.rs` (`rebuild_output_idx`)
- `agentmux-srv/src/server/app_api/blockfile.rs:100-200` (freshness check)
- `frontend/app/view/agent/hooks/useSnapshotPersistence.ts:41,74,133`
- `frontend/app/view/agent/activity/{backfill-tracker,debounced-refresh,subagent-source,dispatch-source}.ts`
- `frontend/app/view/agent/hooks/useSubagentBackfillGate.ts`
- `frontend/app/store/wps.ts:41-48` (unscoped subscribe ⇒ `allscopes: true`)
- Prior work: `REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md`,
  `REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md`,
  `retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md`,
  `retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md`,
  `STATUS_CROSS_CHANNEL_AGENT_OPEN_FULL_APP_FREEZE_2026_08_22.md`
