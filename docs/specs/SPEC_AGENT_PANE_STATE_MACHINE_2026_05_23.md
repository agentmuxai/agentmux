# SPEC — agent-pane turn-phase: discriminated union evolution

**Date:** 2026-05-23
**Author:** AgentA
**Status:** Draft
**Scope:** the agent pane's per-turn lifecycle state in
`frontend/app/store/agent-pane-state/`, the "working" animation, the
interrupt path. This spec **builds on** existing work — it is not a
green-field design.
**Related (must-read first):**
`docs/specs/AGENT_PANE_REDUCER_AUDIT_2026_05_12.md` (the audit this
spec extends), GitHub issue **#728** (the 6 gaps the audit produced),
`docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md` (where this
slice sits in the broader reducer stack), `SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md`
(precedent shape).

---

## 1. Summary

The agent pane is **already reducer-based** — two pure slices
(`agent-pane-state/`, `agent-document/`) following the established
`update(state, command) → { state, events }` shape (per the May 12
audit). This spec proposes one targeted evolution on top of that
foundation: **replace the orthogonal booleans `turnActive` +
`stopping` + `streaming.active` with a single `turnPhase:
TurnPhase` discriminated union** so the legal-state set is exactly
representable and the "working" animation becomes a pure projection
of `turnPhase.kind`.

The bug it targets — *"the working animation breaks on interrupt"* —
is a downstream symptom of the boolean-trio shape: 2³ = 8 storable
combinations, only ~4 of them legal, multiple sources of truth that
can drift mid-transition. The discriminated union makes the illegal
states unrepresentable.

## 2. Where we are today

**Existing module** — `frontend/app/store/agent-pane-state/`
(May 19 commit; live; passing tests):

- `types.ts` — `AgentPaneState { streaming, sessionStats, currentTool, turnTokens, turnActive, stopping, pending, initPhase, initError, lastEventMs }`. Per-pane atoms with cross-atom invariants.
- `reducer.ts` — pure `update(state, command) → { state, events }`.
- `reducer.test.ts` — cross-product transition coverage.

**Existing audit** — `AGENT_PANE_REDUCER_AUDIT_2026_05_12.md` —
identified 6 gaps. Translated into **issue #728**.

**Issue #728 — open, six tracked gaps:**

| # | Gap | This spec's coverage |
|---|---|---|
| 1 | Init phase not covered (`InitState`/`InitQuestion` types exist but no lifecycle) | **Out of scope** — `initPhase: "loading" \| "ready" \| "error"` already in `types.ts`; #728 commands (`InitStart`/`InitReady`/`InitFailed`) close it independently. |
| 2 | No acceptance timeout — `pending[]` linger if `agent-message-accepted` never arrives | **Naturally fits** in the discriminated union — `Submitting` becomes a bounded state with a timeout transition. See §6.2. |
| 3 | Stuck-stream recovery watchdog | **Naturally fits** — `Streaming` becomes a bounded state; `StreamStalled` event transitions to a recovery sub-state or terminal. See §6.3. |
| 4 | `nodeIdSet` rebuilt on remount | **Out of scope** — that's `agent-document/`'s territory. |
| 5 | `DocumentState` outside reducers | **Out of scope** — also `agent-document/`. |
| 6 | Hardcoded `TRUNCATE_GRACE_MS` | **Out of scope** — utility refactor; works with either state shape. |

So this spec **owns gaps 2 and 3** if/when we ship it (cleaner shape
than #728's proposal), is **complementary** to gap 1 (init phase
stays orthogonal), and is **untouched** by gaps 4/5/6 (different
slice / utility).

## 3. Motivation

### The working animation breaks on interrupt

User reports the spinner either keeps animating after Stop, or freezes
mid-frame. The root cause is structural:

- Animation is driven by some derivation of `(turnActive, stopping, streaming.active)`.
- On Stop click, view sets `stopping = true` immediately. RPC fires `SIGINT`. Backend takes a few hundred ms to acknowledge — during which `turnActive` is still `true` and `streaming.active` may still be `true`.
- Different parts of the view read different combinations of those three. They drift out of phase during the interrupt window. The spinner is most-prone because it polls a derived value.

There is no single source of truth. With three booleans there are eight storable combinations, only a subset are legal, and there's no compile-time enforcement of which are which.

### The boolean-trio shape lets illegal states exist

| `turnActive` | `stopping` | `streaming.active` | Legal? |
|:---:|:---:|:---:|---|
| false | false | false | ✓ Idle |
| true  | false | false | ✓ Submitting (no chunks yet) |
| true  | false | true  | ✓ Streaming |
| true  | true  | true  | ✓ Interrupting mid-stream |
| true  | true  | false | ✓ Interrupting after stream ended |
| false | true  | * | **✗ stopping without an active turn** |
| false | false | true  | **✗ streaming without an active turn** |

Type system can't prevent the illegal rows. Runtime invariants in the
reducer can — but every new field added compounds.

## 4. Goals / non-goals

**Goals:**

- Replace `turnActive` + `stopping` + `streaming.active` with a single `turnPhase: TurnPhase` discriminated union. Illegal states unrepresentable at compile time.
- `isWorking(state)` is the only animation driver; it's a pure function of `turnPhase.kind`. Existing direct subscriptions to `controllerstatus` from the spinner go away.
- Every transition out of "busy" is explicit and bounded. The `Interrupting → Done.Interrupted` transition cannot exceed N ms — closes the "animation never stops" symptom by construction.
- Close issue **#728 gaps 2 + 3** (acceptance timeout + stuck-stream watchdog) by folding them into Submitting / Streaming state bounds.

**Non-goals:**

- `initPhase` reorganisation — keep it orthogonal to `turnPhase`. #728 gap 1 closes it on its own.
- `agent-document` changes — separate slice, separate spec needs.
- Streaming-chunk plumbing — `OutputChunk` is just an event into the reducer; the chunk-decoder pipeline (markdown / tool / decision-prompt) is upstream of this slice.
- Conversation-level state — multi-turn history, queued user inputs. Out of scope.

## 5. Proposed evolution

```ts
type TurnPhase =
    | { kind: "Idle" }
    | { kind: "Submitting"; submittedAt: number; pendingContent: string }
    | { kind: "Streaming"; bufferSize: number; toolsActive: number; lastEventMs: number }
    | { kind: "Interrupting"; reason: InterruptReason; sigintSentAt: number }
    | { kind: "Done"; outcome: TurnOutcome; finishedAt: number }
    | { kind: "Disconnected"; lastKind: KindBeforeDisconnect; lastConnectedAt: number; reason: DisconnectReason };

type InterruptReason = "user-stop" | "user-esc" | "submit-timeout" | "stream-stalled";
type TurnOutcome     = "success" | "error" | "interrupted" | "crashed";
type KindBeforeDisconnect = "Submitting" | "Streaming" | "Interrupting";
type DisconnectReason = "stream-unsubscribed" | "transport-error";
```

> **PR F note (2026-05-23):** the `Disconnected` payload was fleshed out
> from `{ lastKind, reason: string }` (PR A stub) to
> `{ lastKind, lastConnectedAt, reason: DisconnectReason }`. `lastKind`
> stays for the "was streaming / was submitting" banner copy;
> `lastConnectedAt = command.at` from the `StreamUnsubscribe` arm so the
> banner can render "disconnected 12s ago"; `reason` tightens to a finite
> literal union so the dispatcher can attach typed reasons going forward
> (`transport-error` is reserved — not wired by any caller in PR F).
> The reducer also expands `TurnEnd`'s `alreadyDone` first-done-wins
> guard to include `Disconnected` (Option A): a late TurnEnd while the
> phase is `Disconnected` is a same-ref no-op (the disconnect IS the
> outcome).

**Field-level changes to `AgentPaneState`:**

| Today | After |
|---|---|
| `turnActive: boolean` | *removed* — derive from `turnPhase.kind !== "Idle" && !== "Done" && !== "Disconnected"` if needed for legacy callers |
| `stopping: boolean` | *removed* — `turnPhase.kind === "Interrupting"` |
| `streaming: { active, agentId, bufferSize, lastEventTime }` | `streaming: { agentId }` (just identity) — `active` becomes `turnPhase.kind === "Streaming"`; `bufferSize` + `lastEventMs` move into the `Streaming` variant payload. |
| `currentTool: string \| null` | *unchanged* — orthogonal to the phase; one tool at a time today, but multi-tool future-compatible via `Streaming.toolsActive`. |
| `initPhase`, `initError` | *unchanged* — separate concern. |
| `sessionStats`, `turnTokens`, `pending` | *unchanged*. |

**`lastEventMs`** moves into the `Streaming` variant payload — it's
only meaningful during a stream, and putting it there means the
"stuck-stream" watchdog (#728 gap 3) reads from a place where the
value is guaranteed live.

### 5.1 Transitions

| From          | Event                                   | To                  | Side-effect |
|---|---|---|---|
| Idle          | `UserSubmitted{content}`                | Submitting          | `SendInputRpc{content}` |
| Submitting    | `BackendAck` *or* `OutputChunk`         | Streaming           | — |
| Submitting    | `SubmitTimeoutElapsed` *(gap 2)*        | Done.Error          | — |
| Submitting    | `UserInterrupted`                       | Interrupting        | `SendSigintRpc` |
| Streaming     | `OutputChunk`                           | Streaming (updated) | — |
| Streaming     | `ToolStarted` / `ToolEnded`             | Streaming (counts)  | — |
| Streaming     | `UserInterrupted`                       | Interrupting        | `SendSigintRpc` |
| Streaming     | `StreamStalled{idleMs}` *(gap 3)*       | Interrupting        | `SendSigintRpc` (reason = stream-stalled) |
| Streaming     | `TurnFinished{outcome}`                 | Done.{outcome}      | — |
| Streaming     | `ControllerStatusChanged{done, exit≠0}` | Done.Crashed        | — |
| Interrupting  | `TurnFinished{outcome}`                 | Done.Interrupted    | — |
| Interrupting  | `ControllerStatusChanged{done}`         | Done.Interrupted    | — |
| Interrupting  | `InterruptTimeoutElapsed`               | Done.Interrupted    | **bounded** force-transition |
| Done          | `UserSubmitted{content}` / `UserRetried`| Submitting          | `SendInputRpc` |
| *any non-Done*| `Disconnected{reason}`                  | Disconnected        | — |
| Disconnected  | `Reconnected`                           | Idle                | — |

### 5.2 Guards (no-ops, idempotent)

- `UserSubmitted` in Submitting / Streaming / Interrupting → no-op (logged). Don't double-send.
- `UserInterrupted` in Idle / Done / Disconnected → no-op.
- `UserInterrupted` in Interrupting → no-op. SIGINT already sent.
- `BackendAck` in any state other than Submitting → no-op.
- `OutputChunk` in Idle / Done / Disconnected → silently dropped.

## 6. How this closes #728's gaps

### 6.1 Gap 1 (init phase) — unchanged

`initPhase` is orthogonal — implement #728's proposed commands
(`InitStart` / `InitReady` / `InitFailed`) directly on the existing
field. No interaction with `turnPhase`.

### 6.2 Gap 2 (acceptance timeout)

In the boolean-trio shape, `pending[]` carried a `localId` and a
`queuedAt`; #728 proposes a separate `PendingMessageExpired` command.

In the discriminated union, `Submitting` *is* the unaccepted state —
the timeout is a transition out of it. The reducer emits
`ScheduleSubmitTimeout { ms: 30_000 }` on entering Submitting; the
view fires `SubmitTimeoutElapsed` if `BackendAck` / `OutputChunk` /
`UserInterrupted` don't arrive first. The transition lands in
`Done.Error` with a typed timeout reason in the outcome payload.

`pending[]` stays a separate field for queued-but-deferred messages
(e.g. user typed faster than RPC ack), independent of the current
turn's phase. #728 gap 2's timeout is now covered by the Submitting
bound; `pending[]` only needs cleanup when its corresponding turn
reaches Done.

### 6.3 Gap 3 (stuck-stream watchdog)

`Streaming` carries `lastEventMs` in its payload. A `setInterval(5s)`
in the view fires `StreamWatchdogTick { nowMs }`; the reducer compares
to the live `Streaming.lastEventMs`. If `nowMs - lastEventMs >= 45_000`
(spec'd in #728), the reducer transitions to `Interrupting` with
`reason: "stream-stalled"` and emits `SendSigintRpc`. Tools downstream
see the same shape as a user-initiated interrupt; UI uses the reason
to surface "stream stalled — interrupted automatically" instead of
"you pressed Stop".

### 6.4 Bonus — `Disconnected`

Not in #728. Today there's no state for "PTY/WS lost mid-turn"; the
pane just stalls. Adding the explicit kind gives the UI a banner
target and a clean reconnect path. Strictly an addition; doesn't
collide with anything in #728.

## 7. The animation projection

```ts
export function isWorking(state: AgentPaneState): boolean {
    switch (state.turnPhase.kind) {
        case "Submitting":
        case "Streaming":
        case "Interrupting":
            return true;
        case "Idle":
        case "Done":
        case "Disconnected":
            return false;
    }
}
```

`agent-pane-state` exports `isWorking` as a selector (sibling of the
existing `realIdentities` / `canSubmit`-style selectors in other
slices). Spinner / input-disabled / Stop-visible / Disconnected-banner
all read derivations of this one source. Removes the direct
`controllerstatus` subscription from the spinner.

## 8. The bounded `Interrupting → Done.Interrupted` invariant

The reducer emits `ScheduleInterruptTimeout { ms: 3_000 }` on entering
Interrupting. The view runs `setTimeout`, dispatches
`InterruptTimeoutElapsed` if no `TurnFinished` /
`ControllerStatusChanged(done)` arrives first. The transition lands in
`Done.Interrupted`. **Three independent paths into Done.Interrupted**
— so the animation can never get stuck in Interrupting.

`SendSigintRpc` is only emitted once (on entry). Subsequent
`UserInterrupted` while already in Interrupting is a no-op.

## 9. Rollout

### 9.1 Sequencing with #728

Two paths, your call:

**Path A — close #728 first, evolve after.** Implement gaps 1-3 as
prescribed in the audit (boolean-trio shape + timeout setTimeouts).
Then take this spec on, refactoring those into the discriminated
union. Pro: smaller PRs, in-flight architecture not disturbed. Con:
the timeout + watchdog code gets re-written when the union lands.

**Path B — evolve first, close #728 gaps in the new shape.** Take
this spec's discriminated union directly; gaps 2 + 3 land naturally
during the implementation. Gap 1 still done independently on
`initPhase`. Pro: no rework. Con: a larger single change.

Recommendation: **Path B**. Discriminated-union evolution is the
shape change; doing gap 2+3 within that change is essentially free.
Gap 1 is independent; ship it alongside or before.

### 9.2 PRs in Path B

| PR | Scope |
|---|---|
| **A** | Add `TurnPhase` type + `turnPhase` field to `AgentPaneState`, dual-write alongside legacy `turnActive` / `stopping` / `streaming.active`. Reducer accepts new + old commands. Tests cover both shapes. |
| **B** | Migrate the view's spinner, input-disabled, Stop-visible to read `isWorking(state)`. Drop spinner's direct `controllerstatus` subscription. |
| **C** | Bounded `Interrupting → Done.Interrupted` — `ScheduleInterruptTimeout` event + view's `setTimeout` wiring. Closes the working-animation-breaks-on-interrupt bug. |
| **D** | Gap 2 — `Submitting` bounded by `SubmitTimeoutElapsed`. |
| **E** | Gap 3 — `Streaming.lastEventMs` in the variant payload; `StreamWatchdogTick` + `StreamStalled → Interrupting`. |
| **F** | Disconnected state + banner UI. |
| **G** | Cleanup — drop the dual-write; remove legacy `turnActive` / `stopping` / `streaming.active` fields. View migrates fully to `turnPhase`. |

### 9.3 #728 gap 1 (init phase) lands independently

`InitStart` / `InitReady` / `InitFailed` commands on the existing
`initPhase` field. Can ship before, during, or after this sequence —
no coupling.

## 10. Tests

Extend `frontend/app/store/agent-pane-state/reducer.test.ts`'s
cross-product. Key invariants the suite must pin:

- **i1**: `isWorking(state)` true ⇔ `state.turnPhase.kind ∈ {Submitting, Streaming, Interrupting}`. No exceptions.
- **i2**: `UserInterrupted` from any working state always lands in `Interrupting` and emits exactly one `SendSigintRpc`.
- **i3**: From `Interrupting`, *any* one of `TurnFinished` / `ControllerStatusChanged(done)` / `InterruptTimeoutElapsed` lands in `Done.Interrupted`. Bounded.
- **i4**: `UserInterrupted` while in `Interrupting` is a no-op — no second `SendSigintRpc`, no state change.
- **i5**: `Submitting` bounded by `SubmitTimeoutElapsed` (gap 2 invariant).
- **i6**: `Streaming.lastEventMs` updated on every `OutputChunk` / `ToolStarted` / `ToolEnded`; `StreamWatchdogTick` with `nowMs - lastEventMs >= 45_000` transitions to `Interrupting` (gap 3 invariant).
- **i7**: Late `OutputChunk` after `Done` / `Disconnected` is silently dropped.
- **i8**: `Disconnected` preserves `lastKind` so the UI banner can say "was streaming" vs "was idle".
- **i9**: Illegal-by-construction states aren't representable — TypeScript ensures.

## 11. Open questions

1. **Second-interrupt → SIGKILL?** Today: no-op (i4). User pressing Stop twice probably wants force-kill. Defer to v2 unless UX complaints arise.
2. **Disconnected granularity** — split backend-gone (sidecar crashed) from network-blip (WS drop)? v1: one state. v2: split.
3. **`currentTool` vs. `Streaming.toolsActive`** — single string vs. count. Claude Code today is one-tool-at-a-time; the count is forward-compatible. Keep both or pick one? Recommendation: keep both, document `currentTool` as "name of the most recent tool" and `toolsActive` as "number currently running" (matches today's behaviour where they're usually 1/0).
4. **Phase B's submit timeout — 30 s** (per #728) or shorter? RPC ack should be sub-second normally; 30 s is generous. Tune in PR D.

## 12. Why this is worth doing

The May 12 audit already identified the multi-source-of-truth shape as
the underlying problem; #728 documents six concrete symptoms. This
spec is the **shape change** that makes most of them
unrepresentable-by-construction rather than runtime-checked. The
interrupt-animation-breaks bug is the most visible one, but it's a
class — `turnActive + stopping` interactions, `streaming.active`
diverging from `turnActive`, and so on. A discriminated union
collapses the class into a single typed enum.

Cost: ~7 PRs (one per row in §9.2), each contained. None destabilise
the in-flight architecture — the dual-write phase (PR A) keeps the
existing API intact while the new shape grows alongside.
