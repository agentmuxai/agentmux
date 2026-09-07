# SPEC: A6 — remove the agent-pane `AgentAtoms` mirror; render from the reducer stores

**Status:** implemented — this PR. A6 of issue #1549 (`docs/analysis/TRACKING_ARCHITECTURE_REFACTOR_A1_A15_2026_06_18.md`). Rides with the code per the no-doc-only-PR rule.
**Date:** 2026-09-06
**Author:** Korp
**Baseline:** `main` @ `c45721354`
**Related:** `SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` (the reducer this exposes), `docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md` (the cascade contract preserved here), `REPORT_LARGE_MIGRATIONS_COMPLETION_AUDIT_2026_09_06.md` §3.1 (why now: `agent-view.tsx` grew 1,282 → 2,730 lines while this sat open), `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md` (the scroll/expansion half of A6, **not** in scope here — see §5).

---

## 1. Problem

The agent pane had **four parallel state systems** for one pane (the A6 board entry). This PR collapses the one that was pure duplication:

`agent-view.tsx` created a bundle of **19 Solid signals** (`createAgentAtoms`, `frontend/app/view/agent/state.ts`), handed **17 of their setters** to the pane-state store through a hand-maintained `AgentPaneProjections` interface plus the document store's `documentSetter`, and the stores wrote each changed field back through the matching setter. The view then rendered from the atoms.

Consequences, all observed in the tree at the baseline:

| Symptom | Evidence |
|---|---|
| Adding a reducer field touched **four** files (`types.ts`, `initialState`, `AgentPaneProjections`, `createAgentAtoms`) plus the 17-line wiring block in `agent-view.tsx` | `agent-pane-state-store.ts:46-128`, `state.ts:43-198`, `agent-view.tsx:759-785` |
| Forgetting one was silent — the field simply never rendered | the projections interface had 11 `?`-optional setters "for back-compat with existing test projections" |
| The view could write a mirrored signal directly, bypassing the reducer that owned the field | `agent-view.tsx:1764` `agentAtoms().detailsOpenAtom[1](true)` and `:1024` `setDetailsOpen(detailsOpen)` — both for a field the reducer owns (`DetailsToggle`/`DetailsExpand`/`DetailsCollapse`), so reducer state and rendered state could disagree |
| Every consumer typed its inputs as `SignalPair<X>` (getter + setter) while only ever reading | 7 hooks/components; none used the setter |

## 2. Design

**The reducer state is the single source; the store owns its reactive read side.**

### 2.1 `AgentPaneView` — one signal per reducer field, generated

`agent-pane-state-store.ts` gives each slot a `FieldSignals` map built by iterating `Object.keys(initialState(agentId))`, and a frozen `AgentPaneView` object whose getters read those signals. After every dispatch, the store compares `prev[key] === next[key]` for each key and writes only the changed ones, inside one `batch()`.

This is exactly the old per-field projection semantics — referential equality per top-level field, fine-grained reactivity — with the wiring generated instead of hand-written. **Adding a field to `AgentPaneState` + `initialState()` makes it reactive with no other edit.** A DEV-only guard warns if the reducer ever produces a key `initialState()` did not declare.

### 2.2 Why not `createStore` + `reconcile`

`launch-flow-store.ts` uses that pattern and it was the first candidate. Rejected: `reconcile` diffs **into the store's existing object tree in place**. After the first dispatch the tree aliases the reducer's own state objects, so a later reconcile mutates a `prev` that `dispatch()`'s `[wave-turn]` diagnostics still compare against — and any `snapshot()` a caller holds. Per-field signals never mutate anything the reducer produced.

### 2.3 Document nodes

`agent-document-store.ts` owns a `createSignal<DocumentNode[]>` per slot (same referential-equality publish as before) and exports `documentNodes(blockId)`.

### 2.4 The model carries both

`AgentPaneModel` gains `readonly state: AgentPaneView` and `readonly document: Accessor<DocumentNode[]>`, populated by `registerPane` from the two stores. `PaneRegistration` shrinks to `{ agentId }`. Both stay readable after `disposed` (last published state) — nothing writes to them except the stores.

### 2.5 Consumers

`SignalPair<X>` props become `Accessor<X>` (`documentNodes`, `turnPhase`, `compacting`, `pendingMessages`) in `useAgentStream`, `useAgentCommands`, `useTurnLifecycle`, `usePendingMessageAcceptance`, `ActivityDock`, `AgentDocumentView`, `commands/types.ts`, `virtualization/state.ts`, `useSnapshotPersistence`. `agent-view.tsx` reads `paneModel.state.<field>` / `paneModel.document()`.

The two out-of-band `detailsOpen` writes become `paneModel.dispatchPane({ type: "DetailsExpand" | "DetailsCollapse" })`.

### 2.6 What stays view-local

`AgentAtoms` keeps exactly one field: `documentStateAtom` (collapse/pin sets, `expandedTools` hold, scroll, selection, filter). No reducer owns that; `AgentDocumentView` and the virtualizer manage it. `state.test.ts` pins that it is the only key.

## 3. Cascade contract — preserved

`LIFECYCLE_DISPATCH_LEAK_2026_05_15.md`'s scenario — a reactive subscriber unmounts the pane synchronously during a dispatch — still fires `CASCADE_DETECTED` naming the changed field(s), and the next hard `dispatch` still throws while `dispatchIfRegistered` still returns `[]`. The store tests now express the subscriber as a `createComputed` over `paneView(id).streaming` rather than a hand-passed setter.

## 4. Tests

- `agent-pane-view.test.ts` (new) — **generic**: the view exposes exactly `initialState()`'s keys; after a run of dispatches every field is identity-equal to `snapshot()`; a dispatch notifies readers of exactly the fields it changed and a no-op notifies none; the view outlives the slot. Plus grep-shaped guards that `AgentPaneProjections`, `documentSetter`, and any `agentAtoms().<reducerField>Atom[0|1]` in `agent-view.tsx` cannot creep back.
- `agent-pane-state-store.test.ts`, `agent-pane-registration.test.ts`, `agent-pane-model.test.ts`, `useAgentCommands.test.ts`, `useTurnLifecycle.test.ts`, `usePendingMessageAcceptance.test.ts`, `login.test.ts`, the three `AgentDocumentVirtualList.*.test.tsx`, `virtualization/state.test.ts`, three swarm tests, `agent-document-store.test.ts`, `state.test.ts` — converted to the new API; assertions that used to count setter calls now read `model.state` / `model.document()`.

## 5. Explicitly out of scope — the other half of A6

The board's A6 also says "unify the dual scroll/expansion bridged by `expansion-source.ts`." That is `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md`'s render-path wiring (Phase 0 shipped: slice + store + tests, "no render-path wiring yet"). It is a separate reducer with its own spec and is not touched here. A6's **acceptance criterion** — "adding a pane state field touches one place; no `AgentAtoms ⇄ AgentPaneState` copy" — is met by this PR; the scroll/expansion unification should be tracked as its own item against that spec rather than left implicit under A6.

## 6. Verification

- `tsc --noEmit`: 0 errors (baseline on `main` also 0).
- vitest over `frontend/app/store`, `frontend/app/view/agent`, `frontend/app/view/swarm`: 168 files / 2,713 tests green; full suite run recorded in the PR.
- ESLint could not be run on this checkout (pre-existing config-load failure, see #3044).
