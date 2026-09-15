# Report: Swarm's tracked-block list goes stale — refresh is decoupled from its own data source

**Status:** proposed
**Date:** 2026-09-15
**Author:** Korp
**Ask (repo owner):** *"I notice that none of the open agents appear in the swarm view ... I opened
this instance with agents that already exited ... when opening a new agent it usually works. Check
if anyone worked on it, and if we need more robust swarm handling, write report to file."* Follow-up:
*"pin down the open questions, then refine the report, then proceed with the robustness plan."*

---

## 0. The short version

**Live-reproduced, not just theorized.** A screenshot of the reporter's actual running instance
(§2) shows the Swarm pane listing **1 of 5** currently-live, actively-responding agent panes
(`Camper`, `Agent3`, `Korp`, `AgentY`, `Agent5` all visibly mid-task in the same window; Swarm shows
only `AgentY`). The `agentmux-srv` log for that same instance proves all of them **did** register a
controller — `resync_controller` fired for every one at boot with the correct `controller_type`, and
again later via forced resyncs when each received a real message. So the backend's source of truth,
`CONTROLLER_REGISTRY`, is not the problem — my first-pass theory (mount-dependent registration, see
§1 for what's still true from it) was **wrong on the "registration never happens" claim**, confirmed
wrong by direct evidence, not superseded speculation.

**What's actually broken:** Swarm's `agent.tracked-blocks` list is fetched **once** on mount
(`SwarmViewModel.loadAll()` → `loadTrackedBlocks()`) and refreshed only by four specific WPS events
(`agent:process-added`, `agent:process-exited`, `agent:reactive-registered`,
`agent:reactive-unregistered`) — **none of which are emitted by the same code path that actually
maintains `CONTROLLER_REGISTRY`.** `agent:process-added`/`-exited` come from `AgentProcessRegistry`,
a *separate*, independently-populated OS-process tracker that emits on a **2-second poll-and-diff
loop**, decoupled entirely from `register_controller`/`resync_controller`. The subscription mechanics
themselves are sound (verified — an unscoped `waveEventSubscribe` does register for
`allscopes: true` server-side, so this isn't a scope-filtering bug). The dependency is just on the
wrong signal: Swarm's *list membership* answer comes from one mechanism
(`CONTROLLER_REGISTRY`/`resync_controller`), but its *refresh trigger* comes from a different,
independently-timed one (`AgentProcessRegistry`'s poller, and `reactive`'s registration events) —
exactly the "two structurally different, unreconciled mechanisms" failure pattern
`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` diagnosed for *discovery* in general,
still present here for *refresh* specifically, one layer down from where that report's fix landed.

**Nobody has worked on this specific gap.** See §4 for the full prior-work search — the July 22
Process Broker consolidation fixed discovery inconsistency but didn't touch (and doesn't discuss)
how Swarm's client-side list gets invalidated/refreshed after its first fetch.

---

## 1. What I confirmed in code (backend registration itself is fine)

1. **`agent.tracked-blocks`'s only data source is the live controller registry.**
   `agentmux-srv/src/server/app_api/agent_io.rs:49-80` (`register_agent_tracked_blocks`) —
   `process_broker.list_agent_panes()` → `agentmux-srv/src/broker/process.rs:300`
   (`list_agent_panes`) → `self.list()` → `blockcontroller::get_all_controllers()`
   (`agentmux-srv/src/backend/blockcontroller/mod.rs:342`), which is a plain clone of the
   `CONTROLLER_REGISTRY` `RwLock<HashMap>` (`mod.rs:279`). Nothing else feeds this RPC.

2. **The registry is populated by `register_controller`, called from `resync_controller`** —
   `mod.rs:292` (`register_controller`), called *before* `ctrl.start(...)` in every controller-type
   branch (`mod.rs:587/603/617/630`), so a controller stays registered even if its process fails to
   actually (re)connect. `resync_controller` runs from the frontend's agent-pane mount sequence
   (`agent-view.tsx`'s `onMount` → `launch-flow.ts` Phase 3, unconditional — no gate on active-tab,
   fresh-vs-restored, or anything else) and again on every message send
   (`server/agent_handlers/input.rs`, `force: true`).

3. **Session restore never touches a controller directly** —
   `agentmux-srv/src/server/service/session_restore.rs`'s `restore_last_session` recreates
   tabs/blocks/meta only; it relies entirely on the frontend mounting those restored panes to trigger
   registration. Confirmed the full `block.meta` (including the controller-type key) round-trips
   through the snapshot unmodified (`snapshot_workspace` captures `{"meta": block.meta}` verbatim,
   `CreateBlock`'s meta is passed straight through on restore) — so this is not a "meta got stripped"
   bug either, a theory this investigation raised and then ruled out.

4. **The frontend mounts eagerly, not lazily, within the currently-open window** —
   `frontend/app/workspace/workspace.tsx`'s `<For each={allTabIds()}>` renders every tab
   unconditionally (inactive ones get `display:none`, not omitted); within a tab, `TileLayout` gives
   every leaf a real `ContentRenderer`, not a lazy placeholder.

## 2. Live reproduction — what's actually happening

Rather than keep reasoning from static code, I checked the reporter's own currently-running instance
directly, since it was already exhibiting the bug.

**The `agentmux-srv` log proves registration succeeded for every affected block.** Six
`resync_controller entry` lines fire in a tight batch at boot (`02:02:16`, same thread, ~2ms apart),
one per restored agent block, every one with `controller_type: "persistent"` — the correct value,
confirming finding 3 above (meta survived restore intact). Six more `force: true` resyncs follow
over the next ~28 minutes as each agent actually received a message during this conversation
(`02:02:54` through `02:30:45`). Per finding 2, every one of these calls registers (or re-registers)
a controller *before* attempting to start it — so `CONTROLLER_REGISTRY` provably has, and has had
throughout, entries for every one of these blocks.

**`mcp__agentmux__FleetList` confirms it from a second, independent angle**: all five agent blocks
(`Korp`, `AgentY`, `Agent3`, `Camper`, `Agent5`) show as `addressable: true` with valid `block_id`s
in the *reactive* registration layer (`backend::reactive`'s own registration map — mechanism #3 from
the July 22 report, a different registry than `CONTROLLER_REGISTRY` but further proof these blocks
are alive and known to the backend, not orphaned data-model rows with no live counterpart).

**A screenshot of the actual window (`mcp__agentmux__CaptureWindow`) shows the Swarm pane at this
exact moment listing exactly one agent — `AgentY`, "161k · idle"** — while `Camper`, `Agent3`,
`Korp`, and `Agent5` are all visible as separate panes in the very same screenshot, one of them
(`Agent3`) mid-tool-call. This is a live, current, directly-observed instance of the reported bug,
not a reconstruction. Notably it is **not** an all-or-nothing failure — 1 of 5 genuinely-tracked
agents does show, ruling out any theory where registration is categorically skipped for restored
blocks (already weakened by findings 1-4 above, now conclusively ruled out by this screenshot).

**Root cause: Swarm's refresh trigger is sourced from the wrong mechanism.**
`swarm-model.ts`'s `loadTrackedBlocks()` runs once at mount (`loadAll()`, called from
`SwarmViewModel`'s constructor) and is otherwise re-triggered **only** by four WPS events:
`agent:process-added`, `agent:process-exited` (both emitted by `AgentProcessRegistry::poll_and_emit`,
`agentmux-srv/src/backend/process_tracker/registry.rs:164` — a **2-second poll-and-diff loop over a
separate, independently-populated process map**, entirely decoupled from `CONTROLLER_REGISTRY`), and
`agent:reactive-registered`/`agent:reactive-unregistered` (`server/reactive.rs:1621/1730` — mechanism
#3, also independent of `CONTROLLER_REGISTRY`). I verified the client-side subscription mechanics are
not the bug: an unscoped `waveEventSubscribe` (`frontend/app/store/wps.ts:43-48`) sets
`allscopes: true` in its server subscription request, so it isn't a scope-filtering miss — these
events, when emitted, do reach Swarm's handler. The problem is structural: **the signal Swarm listens
for to know "recompute my list" is not emitted by the same code path that actually determines what
that list should contain.** Two independent, differently-timed mechanisms (a member-count poller, and
a separate reactive-registration map) stand in as proxies for a third (`CONTROLLER_REGISTRY`) that
never announces its own changes. Any case where the proxy signal doesn't fire for a given block — its
process-tracker entry was already established before the poll cycle in question, its `pid` didn't
change across an observed diff window, or its `reactive` registration state didn't transition in a
way either handler cares about — leaves Swarm's client-side list stale for that block indefinitely,
matching a partial (not all-or-nothing) failure exactly as observed.

## 3. What this rules out from the original investigation

The original pass over this report considered three candidate proximate triggers (multi-window
scope, a resync early-return, and a startup timing race) before this deeper investigation. All three
are now superseded by direct evidence:

- **Multi-window scope** — ruled out: `DiscoverWindows` confirms exactly one AgentMux window is
  running, and it contains all five affected panes.
- **A resync early-return skipping `register_controller`** — ruled out: the log proves
  `resync_controller` completed far enough to log `controller_type: "persistent"` correctly for
  every block, consistent with reaching (and per finding 2, always calling) `register_controller`.
- **Startup timing/race** — ruled out as the *sole* explanation: `Agent5` and `Korp` both had a
  `force: true` resync well after boot (`02:22:28` and `02:05:28`) — long past any plausible initial
  race window — and are still missing from Swarm as of the screenshot taken afterward. A one-time
  race would self-heal; this doesn't, because nothing re-triggers `loadTrackedBlocks()` in a way that
  reliably covers `persistent`-type controllers.

## 4. Prior work — what's already been tried

- **`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`** — found *six* independent,
  unreconciled "is this agent alive" mechanisms and proposed the Process Broker to unify discovery
  behind `CONTROLLER_REGISTRY`. Shipped (`3430a9abf`, PR #2273, then Phase B
  `SPEC_PROCESS_BROKER_PHASE_B_SHELL_ACP_REGISTRATION_2026_07_31.md` for `shell`/`acp` coverage).
  This is good, real progress — it closed the "Agent pane and Swarm pane see different subsets of
  reality" bug class entirely. But by design it made `CONTROLLER_REGISTRY` *the* answer to "does
  this block exist," and that registry was already, and remains, mount-triggered and
  process-lifetime-scoped. The report's own remediation table (line 429) only claims the *activity
  chip* becomes "available regardless of what's mounted where" — it does not claim block *existence*
  in Swarm becomes mount-independent, and nothing in that report or its Phase B follow-up discusses
  server-restart/session-restore reconciliation at all.
- **`SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`**,
  **`SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`** — both about *subagent*
  (Task-tool/Workflow-tool) reconciliation nested under an already-tracked top-level agent block,
  not about the top-level block's own tracked/untracked status. Different layer of the same tree.
- Searched every doc under `docs/specs/`, `docs/reports/`, `docs/retro/` for
  restore/mount/startup/lazy language crossed with `tracked-blocks`/`CONTROLLER_REGISTRY`/
  `get_all_controllers` — no other hits. **This is, as far as the repo's own documentation shows,
  an unaddressed gap**, not a regression in something that used to work.

## 5. Does Swarm need more robust handling? Yes — the robustness plan

The confirmed root cause (§2) is that Swarm's refresh trigger is borrowed from two mechanisms
(`AgentProcessRegistry`'s 2s poller, `reactive`'s registration events) that are independent of, and
not guaranteed to fire in step with, the actual source of truth (`CONTROLLER_REGISTRY`, maintained by
`register_controller`/`delete_controller`). The fix is to couple the signal to the source directly,
plus a bounded self-healing safety net so any *future* gap in that coupling can't reproduce this
class of bug silently and indefinitely again:

1. **Emit an authoritative event directly from the registry's own mutation points.** Add a single
   lightweight WPS event (e.g. `agent:controller-registered` / `agent:controller-removed`, or fold
   into the existing `controllerstatus` broadcast machinery already used elsewhere in this file)
   published from `register_controller` and `delete_controller` themselves
   (`agentmux-srv/src/backend/blockcontroller/mod.rs:292` / `321`) — the exact two functions that
   change what `agent.tracked-blocks` would return. This closes the gap by construction: there is no
   longer a second mechanism standing in as a proxy for the first.
2. **Subscribe `swarm-model.ts`'s `loadTrackedBlocks()` refresh to that new event**, in addition to
   (not instead of — they still carry other useful signal, e.g. finer-grained process-level detail)
   the four it already listens to.
3. **Add a bounded low-frequency safety-net poll** (e.g. every 10-15s, matching the cadence
   `AgentProcessRegistry` itself already uses internally) that re-runs `loadTrackedBlocks()`
   unconditionally while the Swarm pane is mounted. This is deliberately redundant with #1/#2 — its
   only job is to guarantee Swarm self-heals within a bounded time from *any* future gap in the
   event-based path, including ones not yet discovered, without needing another live investigation
   like this one to notice. Cheap: the RPC itself is a single `HashMap` clone plus a `Vec` filter
   (`ProcessBroker::list_agent_panes`), not a per-block probe.
4. Explicitly **not** recommending a boot-time reconciliation pass that pre-registers every persisted
   agent block before any pane mounts — reconsidered from this report's first pass now that live
   evidence shows registration itself already happens correctly and promptly. That would add
   complexity (a new startup-time code path duplicating `resync_controller`'s registration logic) to
   solve a problem that, per §2, isn't actually where the bug lives.
