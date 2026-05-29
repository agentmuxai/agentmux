# Analysis: agent-pane crash + status-bar uptime freeze (2026-05-28 PM)

**Author:** AgentA
**Status:** Diagnosed from live host + sidecar logs. Recurrence of a known crash class — sixth in a 9-day stretch of fixes for this same family.
**Reporter:** user (this session) — "we got a pane crash" + "the timing counter stopped (universal clock on bottom left of status bar)"
**Build:** v0.39.2 (current portable, also the dev build)
**Block id:** `ad77d90f-e99e-4423-94c6-e93b7d4acda5`
**Wall time of crash:** 2026-05-28 19:51:19.270 Z

---

## 1. Symptom

Two simultaneous user-visible problems:

1. **Agent pane crashed** during an active turn — replaced by the `block-error-boundary` fallback. `NotFoundError: Failed to execute 'replaceChild' on 'Node': The node to be replaced is not a child of this node` inside SolidJS's reconciler.
2. **Status-bar uptime counter froze** ("universal clock on bottom left"). This is the `formatUptime(uptimeSecs())` readout in `frontend/app/statusbar/BackendStatus.tsx:141-143`, driven by the `sysinfo` event ts (line 64-79).

The two are almost certainly connected — same dispatch storm.

## 2. Timeline (host log, sub-second resolution)

```
19:51:15.276  user send-message:enter (block=ad77d90, len=20)
19:51:15.276  agent:dispatch:PendingMessageQueued (user-source)
19:51:15.287  agent:dispatch:PendingMessageAccepted (system-source)
19:51:15.287  agent:dispatch:TurnStart
19:51:15.299  agent:virt:partition (streamCount=58, virtCount=84, frontier=toolu_01)
19:51:15.319  agent:dispatch:StreamFlushObserved
19:51:15.854  agent:dispatch:StreamWatchdogTick (5 s cadence, normal)
19:51:18.788  agent:dispatch:TokensIn
19:51:18.805  agent:virt:partition (streamCount=59, virtCount=84)
19:51:18.830  agent:dispatch:StreamFlushObserved
19:51:19.237  agent:virt:partition (streamCount=59, virtCount=84)  ← idle-ish, no growth
19:51:19.256  agent:dispatch:StreamUnsubscribe ← session_end propagated to view
19:51:19.270  ERROR  [block-error-boundary] NotFoundError replaceChild
              (stack=$0e → reconcileArrays → insertExpression → Qme → runComputation)
19:51:19.295  [agentActivity] busyCount=0 panes=[]  ← view layer notices "no longer busy"
19:51:19.295  WARN  [agent-document-store] CASCADE_DETECTED:
              slot disposed mid-dispatch (cmd=StreamFlush, blockId=ad77d90, source=system).
              "A documentAtom subscriber unmounted the pane during this dispatch.
               Subsequent dispatches in the same callback will throw."
19:51:19.305  WARN  [perf] long-task 66.0 ms
19:51:19.701  WARN  [block-error-boundary] (stack-resolved) same error, source-mapped
```

Sidecar log corroborates the natural turn end (not a backend kill):

```
19:51:21.401  INFO  subprocess exited block_id=ad77d90 exit_code=0
19:51:21.401  INFO  agent health transition Healthy → Idle
```

After the crash the host log shows only focus / SetActiveTab events — no further `[fe] agentActivity` heartbeats and no further `sysinfo`-driven uptime ticks. The status bar clock froze at the crash instant because the same `waveEventSubscribe` plumbing the agent pane shares with status-bar widgets stopped delivering for whatever subset got marked disposed in the cascade.

### 2a. New evidence: ALL status-bar perf indicators froze (not just the clock)

User confirmation in the same session: "the clock and all the performance indicators in the status bar are frozen, I never saw that before". This rules out a per-widget render bug and points squarely at the shared `sysinfo` event channel.

Both status-bar consumers subscribe to the same channel:

- `frontend/app/statusbar/BackendStatus.tsx:67-80` — uptime, `eventType: "sysinfo", scope: "local"`
- `frontend/app/statusbar/SystemStats.tsx:47-67` — CPU/GPU/Mem/Net/Disk, `eventType: "sysinfo", scope: "local"`

The dispatch table in `frontend/app/store/wps.ts:126-144`:

```ts
function handleWaveEvent(event: WaveEvent) {
    const subjects = waveEventSubjects.get(event.event);
    if (subjects == null) return;
    for (const scont of subjects) {
        if (isBlank(scont.scope)) { scont.handler(event); continue; }
        if (event.scopes == null) continue;
        if (event.scopes.includes(scont.scope)) scont.handler(event);
    }
}
```

`waveEventSubjects` is a process-global `Map<string, WaveEventSubjectContainer[]>`. Every subscribe pushes a new container with a unique UUID id (line 67-74). Every unsubscribe removes by id (line 91-95) and re-sends the eventsub RPC to the backend (line 102-104). **Crucially**, when the last subscriber for an eventType is removed, the entire key is deleted from the map (line 96-98) and the re-sub message tells the backend "stop sending us this event."

#### Two plausible failure modes

**A. The cascade dropped sysinfo subscribers as collateral damage.** If the cascade unwind during the crash ran `onCleanup` callbacks across the workspace tree — not just the per-block tree — both `BackendStatus` and `SystemStats` would call their `unsub?.()`. The map entry for `"sysinfo"` would be deleted, the eventsub re-sub would notify the backend, and the backend would stop emitting. This matches the observed silence in the host log. SolidJS error handling does NOT normally cascade `onCleanup` past the catching boundary, but if the reconcileArrays error left a half-disposed parent computation, downstream `onCleanup` callbacks could fire in the wrong order — exactly the `feedback_solidjs_reactive_leak` shape.

**B. The status-bar's createEffect / onMount became disposed.** The handler closures stay registered in `waveEventSubjects` (so the backend keeps emitting), but when the event fires and `scont.handler(event)` calls `setUptimeSecs(...)` etc., the signal's parent computation is already disposed — the write succeeds but no render fan-out happens. The user sees frozen values. This would be invisible in logs because there's no error path; the signal write silently goes nowhere.

#### Distinguishing A from B — user-confirmed in-session

The user opened a sysinfo widget pane post-crash and reported "the status bar now is updating". This is decisive:

- The widget pane's mount calls `waveEventSubscribe({ eventType: "sysinfo", scope: "local" })`.
- Every subscribe (line 78-80 in `wps.ts`) — whether or not the eventType already exists in the map — re-issues `updateWaveEventSub(eventType)`, which sends an `eventsub` RPC to the backend.
- The pre-existing status-bar handlers were still registered in `waveEventSubjects` (they would not have started receiving events again otherwise; their subscribe calls only run once at workspace mount and there was no remount).
- Therefore the backend had **stopped emitting `sysinfo`** between the crash and the widget mount, and the widget's re-sub command got it to resume.

This means **failure is on the eventsub aggregator side**, not on the frontend subscriber map. The cascade unwind fired `onCleanup` for some pane-scoped subscriber(s), each calling `updateWaveEventSub` for their eventTypes. Most likely path: an unsubscribe for a `block:<id>`-scoped sysinfo subscriber (if one existed inside the agent pane) caused the backend to recompute its sysinfo emit-list. If the backend's logic treats "0 subscribers for any scope" as "stop emitting", a transient empty state during the unwind would explain it — even though the workspace-scope subscribers are still registered.

**Next code dive:** check the sidecar's eventsub handler logic for sysinfo. If it's keyed only on "is anyone subscribed" without scope-awareness, that's the bug. If it's scope-aware, the bug is in the FE's eventsub message — perhaps it omits the workspace-scope subscribers when it sends the re-sub.

### 2c. Dive results — eventsub aggregator is **NOT** the bug

Read paths:
- `agentmux-srv/src/backend/wps.rs:184-210` — `Broker::subscribe`. Idempotent: calls `unsubscribe_nolock` first then re-adds, so a clean re-sub fully refreshes the (route, event) state.
- `agentmux-srv/src/backend/wps.rs:382-412` — `get_matching_routes`. Scope-aware: walks `all_subs` (allscopes subscribers), `scope_subs` (exact match), `star_subs` (pattern match). Dedups via `HashMap<&str, ()>`.
- `agentmux-srv/src/backend/wps.rs:449-458` — `add_unique` / `add_to_scope_map`. Both dedup-safe. The frontend sending `scopes: ["local", "local"]` collapses to one entry; no double-counting bug.
- `agentmux-srv/src/backend/sysinfo.rs:144-188` — emitter is unconditional `loop { ticker.tick().await; ... broker.publish(event); }`. No "are there subscribers" gate; the broker decides delivery via `get_matching_routes`.
- `agentmux-srv/src/server/websocket.rs:507-549` — eventsub / eventunsub / eventunsuball wired straight through to `broker.subscribe("ws-main", _)`, `broker.unsubscribe("ws-main", _)`, `broker.unsubscribe_all("ws-main")`.
- `agentmux-srv/src/server/websocket.rs:264-270` — **on WS disconnect, the only cleanup is `event_bus.unregister_ws(&conn_id)` + `messagebus.unregister(agent_id)`. NO `broker.unsubscribe_all("ws-main")`.** The broker's subscription state persists across WS reconnects.
- `agentmux-srv/src/backend/eventbus.rs:164-177` — `EventBusBridge::send_event` ignores the `route_id` argument and broadcasts to every registered watch (every WS connection). So as long as broker has "ws-main" subscribed, every connected window gets the event.

**Verdict:** the eventsub aggregator does exactly what it should given correct input. The bug **must** be in the frontend's outbound `eventsub` / `eventunsub` traffic — specifically, the frontend must have sent something during the cascade that told the broker to drop "ws-main" from sysinfo.

### 2d. Where the frontend's outbound traffic goes wrong

`frontend/app/store/wps.ts:36-51`:

```ts
function makeWaveReSubCommand(eventType: string): RpcMessage {
    let subjects = waveEventSubjects.get(eventType);
    if (subjects == null) {
        return { command: "eventunsub", data: eventType };
    }
    let subreq: SubscriptionRequest = { event: eventType, scopes: [], allscopes: false };
    for (const scont of subjects) {
        if (isBlank(scont.scope)) {
            subreq.allscopes = true;
            subreq.scopes = [];
            break;
        }
        subreq.scopes.push(scont.scope);
    }
    return { command: "eventsub", data: subreq };
}
```

The function sends `eventunsub` whenever the map entry is absent. The map entry becomes absent when `waveEventUnsubscribe` removes the last subscriber for that eventType (line 96-98). The cascade ran `onCleanup` callbacks that called `unsub?.()` on a number of subscriptions belonging to the agent pane — none of those should touch sysinfo because the agent pane doesn't subscribe to sysinfo.

**Two remaining candidate mechanisms:**

1. **The cascade disposed `BackendStatus` / `SystemStats` reactive scopes without unmounting their DOM.** SolidJS `onCleanup` is owner-scoped — if some upstream `<Show>` / `<For>` in the workspace tree got reactively invalidated and its computation tree torn down, child owners (including the status-bar components') get disposed. The DOM nodes stay attached because Solid doesn't auto-unmount on owner disposal; it just stops reactivity. The user sees frozen values precisely because the components are "alive" in the DOM but dead in the reactive graph. Their `onCleanup` callbacks ran, `unsub?.()` fired, the sysinfo map entry emptied, `updateWaveEventSub("sysinfo")` sent `eventunsub`, broker removed "ws-main".

2. **The scheduler got stuck.** SolidJS uses a microtask-flushed render queue. An uncaught error during reconciliation can leave queued work that never runs. Subsequent signal writes appear silent — the handler fires the write, the scheduler queues a render task, the task never executes. Opening the sysinfo pane creates new owners + effects which appear to bump the scheduler back into life.

(1) is more consistent with the observation that opening the pane immediately restored ALL status-bar widgets — that would only happen if either (a) the broker resumed emitting OR (b) the scheduler resumed flushing. The pane mount provides (a) by sending a fresh `eventsub`. For (b), it would have to be a coincidence — possible but less parsimonious.

### 2e. Whether (1) and (2) can be distinguished

Add a single line at the BackendStatus handler (line 71 of `frontend/app/statusbar/BackendStatus.tsx`):

```ts
handler: (event) => {
    console.log("[fe] sysinfo handler fired ts=", (event as any)?.data?.ts);
    // ... existing body
}
```

Then re-build and reproduce. On next status-bar freeze:
- If `[fe] sysinfo handler fired` is silent → broker stopped emitting → mechanism (1)
- If `[fe] sysinfo handler fired` continues firing but the displayed value still freezes → scheduler stuck → mechanism (2)

Same instrumentation in `SystemStats.tsx` line 51 for redundancy.

### 2f. Practical fix vs. structural fix

**Practical fix (1 line in `BackendStatus` + 1 line in `SystemStats`):** move the `waveEventSubscribe` call from `onMount` to a top-level call inside the component function (so it runs once at module-level if the file uses a singleton component, or hoist the subscription out of the component lifecycle altogether). This guarantees the subscription survives reactive-tree damage. Tradeoff: leaks the subscription if the workspace ever truly unmounts. Acceptable for a singleton workspace root.

**Structural fix:** add a `waveEventSubscribePersistent` helper that hooks into a root-level dispose registry, independent of the calling component's owner. Status-bar and other workspace-singleton subscriptions opt in. Tradeoff: another API surface; needs care for reconnect semantics. Cleaner long-term.

Either fix isolates status-bar telemetry from the agent-pane crash blast radius. **Neither fixes the underlying agent-pane crash** — that still wants the dispatcher-level "no writes inside writes" structural fix from §6.

### 2b. Implication

The cascade damage reaches beyond the per-block boundary. That makes the structural fix (queue dispatches; disallow writes inside writes) materially more important than the prior 5 fixes implied — every one of those treated this as a per-pane render bug. The status-bar freeze proves it's actually a workspace-root reactive-graph integrity bug. **The pane crash is a *symptom*; the integrity bug is the bug.**

## 3. Root cause (this incident, in the context of the broader pattern)

This is a recurrence of the `CASCADE_DETECTED` family. The cascade detector itself was added in PR #878 specifically to surface this scenario:

> *"A documentAtom subscriber unmounted the pane during this dispatch. Subsequent dispatches in the same callback will throw."*

The sequence that produced this incident:

1. Backend emits `session_end` (subprocess exited cleanly, exit 0).
2. View dispatches `StreamUnsubscribe`, which the reducer translates into `turnPhase = Done`.
3. **Inside the same render tick**, a follow-up `StreamFlush` ships (most likely from the buffered text-delta queue that the post-Unsubscribe code path drains).
4. The `Done` phase transition causes one of the conditionally-mounted view sections (something gated on `isWorking` / `turnPhase.kind`) to unmount — disposing a `documentAtom` subscriber.
5. The buffered `StreamFlush` from step 3 then runs against that already-disposed subscriber slot. SolidJS's `reconcileArrays` walks the prior DOM children, finds a row whose `parentNode` is no longer the virtualizer container, and throws `replaceChild`.
6. `block-error-boundary` catches it and renders the fallback. The pane "crashed" from the user's perspective.

The downstream effect — uptime/clock freeze — is consistent with the cascade tearing down enough of the shared subscription graph that `sysinfo` no longer reaches the status-bar widgets either. (Confirmation needs DevTools, but the host log goes silent on `agentActivity` ticks the moment the crash fires.)

## 4. What's already been done about this crash class (last 9 days)

In rough chronological order — all targeting `replaceChild` / `reconcileArrays` errors on the agent pane during streaming:

| Commit | PR | Date | Fix |
|---|---|---|---|
| `49b8b2ff` | — | 2026-05-26 | Drop `TurnStart` auto-collapse — the same `CASCADE_DETECTED` signature. PR #1068 had added `detailsOpen: false` to the TurnStart reducer arm; that flip raced StreamFlush. |
| `68d9c46e` | — | 2026-05-26 | Swap `<For each={virtualizer.getVirtualItems()}>` → `<Index ...>`. TanStack returns fresh VirtualItem objects each render; Solid's `<For>` re-keyed everything → reconcile-from-scratch every tick. |
| `e224fb95` | #1101 | 2026-05-27 | Per-instance `idPrefix` for stream-parser node ids (id-collision render gap). |
| `768b62a6` | #1101r2 | 2026-05-27 | Codex P1 follow-up: replace random prefix with deterministic snapshot skip-set. |
| `fb2078f1` | #1104 | 2026-05-27 | Scrub orphan in-progress nodes on session reopen. |
| `dfbb2616` | #1122 | 2026-05-28 (AM) | Process newNodes before updatedNodes in StreamFlush, so same-batch updates aren't dropped as orphans. (Author: AgentY.) |

That makes today's incident the **seventh** in this class. The fix list above closed every known reproducer the team encountered through 2026-05-28 AM — but the `StreamUnsubscribe → StreamFlush → cascade` ordering (this incident) is a path none of those targeted. **49b8b2ff** is the closest analog: it removed an unmount-causing reducer write from `TurnStart`. The same surgery is needed on the `StreamUnsubscribe` path.

## 5. Hypothesis for the offending unmount

`agent-view.tsx` has several conditionally-mounted sub-trees that key off `turnPhase` / `isWorking` / `pendingMessages`. Each one is a candidate for the "subscriber unmounted mid-dispatch" cascade. Highest-suspicion set:

1. **`AgentDisconnectedBanner`** (`agent-view.tsx:836`) — mounts when `turnPhase.kind === "Disconnected"`. If `StreamUnsubscribe` first flips the phase to `Disconnected` (not `Done`) and then a buffered flush runs, the banner mounting inserts DOM into the same conditional tree that the flush is reconciling against.
2. **The pending-zone "Send now" button** (`PendingMessagesPanel`, gated on `workingFromPhase`). Its visibility flips on every Submitting↔Streaming↔Done transition — same family as the bug we are about to fix in [ANALYSIS_SEND_NOW_FLASH](./ANALYSIS_SEND_NOW_FLASH_2026_05_28.md). If a final StreamFlush coincides with the phase leaving the working set, the conditional inside `PendingMessagesPanel` unmounts mid-dispatch.
3. **The composer details panel** (`<Show when={detailsOpenAtom()}>`). Same shape as 49b8b2ff if any reducer arm in the Unsubscribe path writes `detailsOpen`.

The render trail captured in the crash payload shows no `DetailsToggle` command immediately before the crash, so (3) is the least likely. The `agent:dispatch:StreamUnsubscribe` literal-string match plus the absence of any `Disconnected`-related signal in the trail makes (1) the leading hypothesis. (2) is also live; the timing-jitter window on Submitting/Streaming/Interrupting transitions is exactly where this class of bug breeds.

## 6. Proposed next actions (priority order)

1. **Defer further dispatches scheduled in the same task as `StreamUnsubscribe`.** This is the architectural fix — the cascade exists because the dispatcher allows a write inside a write. The reducer queue should drain `StreamUnsubscribe` to completion, run lifecycle reactions, and only then permit the buffered `StreamFlush`. (See `frontend/app/store/agent-pane-state/dispatch-queue.ts` or equivalent if it exists; otherwise this is a new layer.)
2. **Add a regression test reproducing this incident's exact dispatch sequence:** `TurnStart → StreamFlush(node_X) → StreamFlush(node_X+1) → StreamUnsubscribe → buffered StreamFlush`. Assert that the buffered StreamFlush dispatches against a *still-mounted* subscriber, or is dropped if the subscriber is gone — but never the in-between state that crashes here.
3. **Audit every `<Show when={...}>` inside `agent-view.tsx` that keys off `turnPhase` or pendingMessages.** Each is a cascade vector if its predicate flips during a flush. The candidates list above is a starting point — but the audit should be exhaustive.
4. **Diagnose the status-bar uptime freeze separately.** Verify with a `console.log` inside the `BackendStatus.tsx` sysinfo handler whether events stop entirely or just become rare. If they stop, it's a subscription-graph teardown (and would be fixed by item 1). If they continue but the handler doesn't run, it's a Jotai/store-level issue.

## 7. Why a one-off fix is the wrong call here

PRs #49b8b2ff / #68d9c46e / #1101 / #1104 / #1122 are five fixes for this same crash class in 12 days. Each one closed *its* reproducer. None addressed the underlying invariant: **the reducer dispatcher allows a write inside a write, and the agent view has too many subscribers gated on transient predicates that flip mid-dispatch.** Patching the next one (e.g. "don't unmount AgentDisconnectedBanner during StreamUnsubscribe") buys days. Closing the invariant — either by queuing dispatches or by collapsing the gating predicates onto a single stable phase — buys years.

The user's "we had a lot of work dedicated to this sort of crash" framing fits exactly: each fix has been local, none has been structural. The right next move is the dispatch-queue / write-during-write fix, not another patch in this layer.

## 8. Open questions for the user

- Was the affected pane idle when the crash happened, or were you actively reading the response as it streamed? (The exit_code=0 says the turn finished cleanly; the question is whether your eyes were on it.)
- Did the status bar's other readouts (CPU / Mem / Net) also stop, or just the uptime? (Tells us whether the entire sysinfo channel froze, vs only the uptime subscription.)
- Did the crash recover (i.e. did clicking "Reload" in the error boundary restore the pane), or did the whole window need to come back?

## 9. Artifacts

- Host log: `~/.agentmux/channels/stable/logs/agentmux-host-v0.39.2.log.2026-05-28`
- Sidecar log: `~/.agentmux/logs/agentmuxsrv-v0.39.2.log.2026-05-28`
- Render trail captured in the crash payload (lines 6–7 of `tool-results/bnvjz6lmh.txt`)
- Stack trace (resolved): `$0e → reconcileArrays → insertExpression → Qme → runComputation`
