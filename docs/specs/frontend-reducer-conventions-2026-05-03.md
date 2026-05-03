# Frontend Reducer Conventions

**Date:** 2026-05-03
**Status:** Draft — answers needed on §10 open questions before any new slice is built.
**Reads-this-first:**
- `docs/specs/frontend-reducer-architecture-2026-05-03.md` — the spec roadmap; this is spec #2 of 8
- `docs/specs/agent-pane-document-reducer-2026-05-03.md` — slice #1, shipped as PR #681; serves as the de facto template
- `frontend/app/store/launcher-event-reducer.ts` — the only pre-existing frontend reducer, single-file pattern
- `agentmux-srv/src/reducer.rs` — canonical Rust reducer style we're echoing

## Purpose

Define the shared shape every frontend reducer slice follows so we don't accumulate inconsistencies as slices #3–#8 are written. This document is **prescriptive** (when you write a new slice, do it like this) but does NOT mandate retroactive uniformity — `launcher-event-reducer.ts` predates this spec and gets harmonized in slice #6.

This is a conventions spec. No code lands from this doc directly. Code lands from the slices that follow it.

## 1. The reducer function

A reducer slice exports a single pure function:

```ts
function update(
    state: SliceState,
    command: SliceCommand,
    nowMs?: number,                    // optional time injection for tests
): { state: SliceState; events: SliceEvent[] };
```

**Rules:**
- Pure: no I/O, no async, no global side effects, no console.log inside the reducer.
- Snapshot-and-drop: never holds a reference to the input state after returning. The returned state is either a new object or — for true no-ops — the same reference.
- Returns the new state plus an array of events the command produced (typically 0 or 1 event).
- `nowMs` parameter for time-dependent logic (suppression windows etc.) so tests can deterministically advance time. Defaults to `Date.now()`.

**Why pure:** matches the Rust reducers' contract. A pure function is trivially auditable, replayable, and testable. Side-effects move OUT to the dispatch layer.

## 2. Command and event types

Every slice defines two discriminated unions:

```ts
type SliceCommand =
    | { type: "DoX"; payload: ... }
    | { type: "DoY"; payload: ... };

type SliceEvent =
    | { type: "x-applied"; ... }
    | { type: "x-suppressed"; reason: string; ... }
    | { type: "y-completed"; ... };
```

**Rules:**
- Command names use **PascalCase verb-phrases** (`SessionStart`, `StreamFlush`, `UserClear`)
- Event names use **kebab-case past-tense facts** (`session-started`, `stream-flushed`, `truncate-suppressed`)
- The split mirrors how the Rust reducers express `Command` vs `Event`. The asymmetry is intentional: commands describe intent ("do X"), events describe outcome ("X happened" / "X was suppressed").
- Suppressed/dropped commands MUST emit a "negative" event (e.g. `truncate-suppressed`, `update-dropped`) so audit logs reflect the decision, not just successful mutations.

## 3. State shape

```ts
interface SliceState {
    // ... domain fields
}

const initialState = (): SliceState => ({ ... });
```

**Rules:**
- `initialState()` is a factory function, not a frozen const — Solid signals don't enjoy shared frozen objects, and tests want fresh state per case.
- State is **immutable**: each command application returns a new object (or the same reference if nothing changed).
- Indices/derived data (Map<id, position>, etc.) generally **not** stored — derived per-command from the source-of-truth fields. Storing them invites the index-out-of-sync class of bugs we just fixed in agent-document. Only store when the rebuild cost is genuinely a problem (typically when state has thousands of items).

## 4. The dispatch layer

For per-instance slices (one cell per blockId, paneId, etc.):

```ts
// Per-key state cell, holding both the reducer state and the projection
// setter that the cell owns.
interface Slot {
    state: SliceState;
    project: (next: SliceState) => void;   // typically writes to a Solid signal
}

const slots = new Map<string, Slot>();

export function registerSlot(key: string, project: (s: SliceState) => void): void;
export function unregisterSlot(key: string): void;
export function dispatch(key: string, command: SliceCommand): SliceEvent[];
export function snapshot(key: string): SliceState | null;       // diagnostics + tests
```

For **global slices** (no key — one state cell for the whole app), drop the Map and key parameter.

**Rules:**
- `registerSlot` is called **synchronously during component-body execution**, not in `onMount`. Reason: hooks dispatching from their own `onMount` would race the registration. This is a hard rule — codex P1 on PR #681 caught the violation.
- `unregisterSlot` is called from `onCleanup`. Failure to unregister leaks a state cell.
- `dispatch` **throws** on unregistered key. Silent drops are the bug we just spent two PRs preventing.
- `snapshot` is a window for tests + the diagnostics panel. Don't use it for app logic — it bypasses reactivity.

## 5. Atom projection

Components keep using their existing Solid signals. The slot's `project` callback writes through them on each command application. Components don't talk to the slot directly:

```ts
// OK — component reads the projection
const [doc] = agentAtoms().documentAtom;
return <For each={doc()}>{...}</For>;

// OK — hook dispatches a command
dispatch(blockId, { type: "StreamFlush", ... });

// NOT OK — component writes the projection directly, bypassing slot.state
setDocument([...]);  // reducer's slot.state is now stale
```

The projection is **write-only from the slot's perspective**: nothing else writes the atom. Slice #1 (PR #681) caught two violations of this rule (handleDecide, onLoginSuccess) and migrated them to dispatch. Future PRs that introduce new mutators must dispatch.

**Read-only access to a per-slot projection from outside is fine** — components rendering the doc, hooks computing derived values from the doc, etc. Only writes need to go through dispatch.

## 6. Echo-loop guard (inbound events)

For slices that mirror an upstream reducer (srv, launcher, host), the dispatch layer needs an `applyingRemote` flag pattern (modeled on `launcher-event-reducer.ts:54`):

```ts
let applyingRemote = false;

export function applyRemoteEvent(event: RemoteEvent): void {
    applyingRemote = true;
    try {
        dispatch(event.key, translateRemoteEvent(event));
    } finally {
        applyingRemote = false;
    }
}

export function isApplyingRemoteEvent(): boolean {
    return applyingRemote;
}
```

Outbound command paths check the flag before re-emitting the change upstream. Without this, a state change from srv triggers a frontend command which sends back to srv which echoes another event which triggers another command — infinite loop.

**Slices that don't mirror upstream state can skip this** (the agent-document slice has no upstream — the document IS frontend-only).

## 7. Audit log

Each slice keeps a ring buffer of the last N events (default 200, per slice). Emitted via the dispatch layer's `onEvent` sink. The diagnostics panel will surface these in a future PR.

```ts
const eventLog: { key: string; event: SliceEvent; at: number }[] = [];

function recordEvent(key: string, event: SliceEvent) {
    eventLog.push({ key, event, at: Date.now() });
    if (eventLog.length > 200) eventLog.shift();
}
```

The agent-document slice currently logs significant events to `console.warn` (suppressed truncates, dropped updates) but doesn't ring-buffer. Adding the buffer is a follow-up.

## 8. Tests

Three layers:

### Reducer tests (the primary safety net)
- Pure, table-driven.
- One file per slice: `<slice>/reducer.test.ts`.
- Cover: each command's happy path; each invariant (suppression, dedup, dropped); purity (input not mutated, no-op returns same reference).
- Should run in <5 seconds per slice.

### Dispatch tests
- Optional but recommended for slices with non-trivial slot lifecycle.
- Cover: register/dispatch/unregister cycle; throw-on-unregistered-dispatch; project callback called with new state.

### Slice-specific tests
- For mirror slices: echo-loop guard works (dispatches inside `applyRemoteEvent` don't re-emit upstream).
- For async-coordinated slices: pending command tracking + rollback (deferred until any slice actually needs this).

## 9. File layout

```
frontend/app/store/<slice-name>/
    reducer.ts               # pure update() + helpers
    types.ts                 # State, Command, Event, initialState
    reducer.test.ts          # table-driven reducer tests
frontend/app/store/<slice-name>-store.ts   # dispatch layer, slot map, public API
```

The split between `<slice>/` (the pure core) and `<slice>-store.ts` (the dispatch layer) lets tests import the pure reducer without dragging in atom dependencies.

For tiny slices (no slot map, single global state), the store file may be redundant — collapse the dispatch into the reducer module's index.

## 10. Open architectural questions — answers proposed

### Q1. Single bus vs. per-slice dispatch?

**Proposed answer:** **per-slice dispatch.** Each slice exports its own `dispatch(key, command)` function. The agent-document slice does this; launcher-event-reducer does this; new slices follow suit.

**Rationale:** A single bus would force every command into a discriminated union spanning all slices, which adds friction without adding value — slices have no need to coordinate at the dispatch layer. The audit log can be made global by sharing a recordEvent function across slices. We keep the dispatch shape symmetric with the Rust reducers (each crate has its own dispatch, no super-bus).

**Reconsider when:** slices need to dispatch atomically across each other (e.g. "select tab AND focus pane" as one transactional unit). Today no use case for this.

### Q2. Inbound event routing — single subscription or per-slice?

**Proposed answer:** **per-slice subscription.** Each mirror-slice subscribes to its own upstream channel (wstore for srv-state slices; launcher-events signal for launcher-state slices).

**Rationale:** Mirrors what `launcher-event-reducer.ts` already does. A single subscription router would need to know which events route to which slice, duplicating the slice's own knowledge of its concern. Per-slice keeps slices independent.

**Reconsider when:** event ordering across slices becomes load-bearing (e.g. tab-switch must apply before pane-focus to avoid flicker). Solvable with explicit ordering hints if needed.

### Q3. Async commands — how do they fit a synchronous dispatch model?

**Proposed answer:** **synchronous dispatch always; async work happens before or after.**

For "create pane" (which needs an srv RPC):
1. Caller dispatches a local command if optimistic UI is needed (e.g. `PaneCreatePending { localId, blockDef }`)
2. Caller fires the srv RPC
3. On srv response (async), caller dispatches a confirm command (`PaneCreateConfirmed { localId, srvId }`) or a rollback (`PaneCreateRejected { localId, error }`)

The reducer never sees promises. The `localId` correlates the optimistic and confirming commands.

**Rationale:** Mirrors the Rust reducer pattern (sagas live outside the reducer). Keeps the reducer pure. Optimistic-UI is opt-in per command, not framework-level.

**Reconsider when:** this rollback boilerplate becomes painful in practice — could justify a small "pending command tracker" helper. Defer until 2+ slices need it.

### Q4. Where does policy enforcement live (e.g. "this agent may not delete that pane")?

**Proposed answer:** **in the slice that owns the resource, not in a shared bus.**

A slice that owns "panes" (per spec #8) gates `PaneDelete` based on policy in its reducer. The reducer can read a permission table from state (which itself was populated via another command). No middleware, no AOP magic.

**Rationale:** Policy is domain-specific. Putting it in the reducer makes it auditable + testable like any other invariant. Spec #3 (frontend-command-bus) was originally framed as a chokepoint for policy; on reflection the chokepoint is the dispatch function itself, not a separate bus layer.

**Reconsider when:** policy genuinely cuts across slices (e.g. "this agent is sandboxed; deny all UI mutations from it"). At that point, an outer wrapper around dispatch (a true middleware) might be warranted. Today no slice has cross-cutting policy.

### Q5. Optimistic updates — per-slice or framework-level?

**Proposed answer:** **per-slice, opt-in.**

Slices that need optimistic UI implement it via the pending-command pattern (Q3). No framework support today.

**Rationale:** Most slices won't need optimistic updates (srv is local, latency is small). Building a framework feature for a non-pressing problem is over-design.

**Reconsider when:** 2+ slices need optimistic + rollback and the duplication becomes obvious.

## 11. What this spec does NOT establish

- A central frontend store object. Slices are independent modules; no `appStore.subscribe()` etc.
- Middleware / interceptors / decorators around dispatch. No use case yet.
- Time-travel debugging. Audit log gives us the data; UI is a future feature.
- Type-level enforcement that "all writes go through dispatch" (no compile-time guarantee — relies on convention + code review).

## 12. Sequencing impact on slices #3–#8

With these conventions in place:
- **Slice #3** (frontend-command-bus): partly DESCOPED. Per Q1+Q4, no separate bus is needed — each slice's `dispatch` is the chokepoint for its domain. The remaining concern in #3 (audit + cross-cutting agent-action attribution) shrinks to "global event log helper" + "command-source tagging" (`{ source: 'user' | 'agent:<id>' | 'system' }`). Will rewrite spec #3 once #2 is approved.
- **Slice #4** (agent-pane-state-reducer): straightforwardly follows §1–§9.
- **Slice #5** (frontend-layout-reducer): mirror slice; uses the echo-loop guard from §6.
- **Slice #6** (launcher-event-reducer convergence): mostly already follows the conventions; tightens slot lifecycle + audit log.
- **Slice #7** (tab-state-reducer): mirror slice; same as #5.
- **Slice #8** (pane-tree-reducer): wait for srv E.4.B as before.

## 13. Decision log

This section will accumulate decisions made in subsequent slices that affect the conventions. (Empty at draft time.)
