# Agent pane reducer-coverage audit

**Date:** 2026-05-12
**Owner:** AgentA
**Driving question:** *"Should we first migrate the agent pane to 100% reducer before building the session-replay framework?"*

---

## 1. TL;DR

**No migration needed. The agent pane is already 100% reducer-routed for every state cell that affects rendered output.**

The audit reveals an architecture that wasn't obvious from the surface:

- `state.ts` looks like local SolidJS signal state, but its setters are passed to `registerAgentPaneStatePane` — they're slot-store-writable signals.
- `useAgentStream.ts` explicitly documents the contract: *"Read-side accessors only — all writes route through dispatchPane"* (line 87).
- The reducer dispatches (`dispatchPane`, `dispatchDoc`) are the only write path; signals are the *reactive projection layer*, not independent state.

The previous reading ("11 signals not in reducer, need to migrate") was wrong. Those 11 signals are **outputs** of the reducer, not parallel state.

**The session-replay framework spec (`SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md`) can proceed as written.** The single `recordDispatch` tap captures every rendering-relevant state change.

---

## 2. Architecture (as-built)

```
┌─────────────────────────────────────────────────────────────┐
│  Write path (single)                                        │
│                                                             │
│  useAgentStream / commands / hooks                          │
│              ↓                                              │
│        dispatchPane / dispatchDoc                           │
│              ↓                                              │
│        slot store reducer                                   │
│              ↓                                              │
│        (recordDispatch audit ring)  ← REPLAY TAP            │
│              ↓                                              │
│        signal setter                                        │
│                                                             │
│  Read path (reactive)                                       │
│                                                             │
│        signal accessor → component → DOM                    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

`state.ts` produces a struct of `[Accessor, Setter]` pairs. The setters are handed to `registerAgentPaneStatePane` on mount (`agent-view.tsx:119-128`). From that point forward, the slot-store reducer owns when setters fire. Components consume the accessors, never the setters.

This is the **store-of-truth + reactive projection** pattern — clean and what we want.

---

## 3. Signal-by-signal audit

### `frontend/app/view/agent/state.ts` (11 cells)

| Cell | Reducer-routed? | Notes |
|---|---|---|
| `documentAtom: DocumentNode[]` | ✅ via agent-document slot store | `dispatchDoc` commands: `SessionStart`, `NodesAppended`, `NodesUpdated`, `HistoryLoaded`, `ToolChunkAppend`, etc. |
| `documentStateAtom: DocumentState` | ✅ same slot store | collapsed/pinned/scroll/filter — local UI prefs via reducer |
| `streamingStateAtom: StreamingState` | ✅ via agent-pane-state | `StreamSubscribe` / `StreamUnsubscribe` |
| `sessionStatsAtom: SessionStats \| null` | ✅ same | `TurnEnd` (merges stats) |
| `currentToolAtom: string \| null` | ✅ same | `ToolStart` / `ToolEnd` |
| `turnTokensAtom: TurnTokens \| null` | ✅ same | `TokensIn` / `TokensOut` |
| `turnActiveAtom: boolean` | ✅ same | `TurnStart` / `TurnEnd` / `TurnReset` |
| `stoppingAtom: boolean` | ✅ same | cascades from `TurnEnd` |
| `pendingMessagesAtom: PendingMessage[]` | ✅ same | `MessageQueued` / `MessageAccepted` |
| `initPhaseAtom: InitPhase` | ✅ same | `InitReady` / `InitFailed` |

**Verdict:** 100% reducer-routed. The presence of `setters` in `state.ts` is misleading at first glance — they're consumed by the slot store, not called from elsewhere.

**Sanity check** (`grep` from this audit):
- `setTurnActive`, `setCurrentTool`, `setStopping`, `setSessionStats`, `setInitPhase`, `setStreaming`, `setPending` — called only in `state.test.ts`.
- `useAgentStream.ts` has zero direct setter calls; 20+ `dispatchPane` / `dispatchDoc` calls.

### `frontend/app/view/agent/hooks/useAgentControllerStatus.ts` (6 cells)

| Cell | Reducer-routed? | Why it's OK |
|---|---|---|
| `authUrl: string \| null` | ❌ local signal | OAuth URL during login. Transient — clears on flow end. |
| `canRetry: boolean` | ❌ local signal | "Retry login" button affordance. Local to retry UI. |
| `flowRunning: boolean` | ❌ local signal | Guards re-entry into `startLaunchFlow`. Lifecycle-local. |
| `agentReady: boolean` | ❌ local signal | Spinner gate. Set once on launch success. |
| `loginWaiting: boolean` | ❌ local signal | OAuth polling phase. Resets per flow. |
| `loginCancelled: bool` (let var) | ❌ local mutable | Cancellation flag. Read in poll loop. |

**Verdict:** Launch / OAuth lifecycle state. **Separate concern from the conversation transcript** — these don't appear in the rendered agent document. They drive the launch spinner and the OAuth modal.

**Recommendation:** *do not migrate.* Reasoning:
1. These cells reset every launch — they're transient by design.
2. They have no cross-cell invariants (each is independent).
3. For session-replay, **launch is out of fixture scope** (replays start *after* the agent is ready; the auth flow is mocked / pre-seeded via OAuth credentials).
4. Migrating them into a reducer would add ceremony without unlocking anything (no time-travel benefit, no replay value, no audit benefit).

If we later want to test the launch flow itself (e.g., "OAuth timeout → retry button appears"), that's a separate fixture domain — not the same `.session.ndjson` as conversation replay.

### Other component-local signals (sampled)

| File | Count | Affects replay? |
|---|---|---|
| `AgentLaunchModal.tsx` | 10 | No — modal form state (instance name, runtime radio, identity/memory dropdowns). Modal isn't part of the running pane. |
| `AgentPicker.tsx` | 7 | No — picker UI (search filter, selected card). Pre-launch. |
| `AgentDecisionPanel.tsx` | 7 | No — permission-decision flow. UI-ephemeral. The DECISIONS themselves are dispatched. |
| `AgentFooter.tsx` | 6 | No — footer toggle states. |
| `AgentControlBar.tsx` | 5 | No — control-bar UI. |
| `useSessionDigest.ts` | 5 | Borderline — session summary view. Likely derives from the document store; needs verification only if we want digest-replay tests. |

**Verdict:** UI-ephemeral. Not in fixture scope. Don't migrate.

---

## 4. Why this confused the initial read

The grep for `createSignal` returns 60+ hits across the agent pane, which *looks* like a lot of non-reducer state. But:

- The 11 signals in `state.ts` are slot-store outputs (setters consumed by the store).
- The signals in `useAgentControllerStatus` are launch lifecycle, not session state.
- The signals in modal/picker/footer/etc. are local UI state.

**`createSignal` is the SolidJS primitive — its presence doesn't imply "non-reducer state".** It implies "reactive projection". The actual question is *who writes to the setter*. The audit answer: the slot store writes to `state.ts` setters; nothing else does.

---

## 5. Recommendations

### Do

1. **Proceed with the session-replay framework as spec'd.** The single `recordDispatch` tap is sufficient for capturing every rendering-relevant state change.

2. **Document the bridge architecture.** Add a doc comment at the top of `state.ts` that says: *"This file is the reactive projection layer of `agent-pane-state` + `agent-document` slot stores. Setters are consumed by `registerAgentPaneStatePane` / `registerAgentDocPane`; nothing else may call them. Accessors are the read API for components."* Prevents the same confusion next time.

3. **Add a lint rule** (or grep CI guard) that flags any direct setter call on `state.ts` atoms outside the registration sites. Today the only violator would be `state.test.ts`, which is allowed.

### Don't

1. **Don't migrate `useAgentControllerStatus`.** Launch state is transient and out of replay scope.
2. **Don't migrate modal/picker/footer signals.** UI-ephemeral, not in scope.
3. **Don't add reducer ceremony "for completeness".** The current line is principled: state that affects the rendered conversation tree is reducer-routed; UI affordances around it are local.

### Defer

1. **Launch-flow replay.** Future work if we want to test the OAuth + agent-ready transitions. Would need a parallel `.launch.ndjson` fixture domain. Out of scope for v1 session replay.
2. **Sub-agent recursion replay.** Per session-replay spec §9. v2.

---

## 6. Action items (small, scoped)

| Item | LoC | PR fit |
|---|---|---|
| Add architecture comment to `state.ts` | ~15 | Rides with session-replay implementation PR |
| Add architecture comment to `useAgentStream.ts:87` (expand the existing one) | ~5 | Same PR |
| Optional: lint rule / CI grep for setter-misuse | ~30 | Separate small PR |

Total: < 1 hour of work. Not a "migration phase" — just architecture documentation that prevents future people (and future-us) from re-doing this audit.

---

## 7. Conclusion

The previous answer ("focused migration of `state.ts` + `useAgentControllerStatus` to reducer first") was based on a surface read of `createSignal` counts. The audit shows:

- `state.ts` is **not parallel state** — it's the reactive output of the slot stores.
- `useAgentControllerStatus` is **out of replay scope** — launch lifecycle, not session state.

The agent pane is, by the criterion that matters (every rendering-state write goes through a reducer dispatch), already 100% reducer-routed. We can proceed straight to the session-replay framework with no migration in front of it.

---

## 8. Cross-references

- Session-replay spec: [`docs/specs/SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md`](../specs/SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md)
- Slot store + `recordDispatch` audit: PR #764
- agent-pane-state reducer: `frontend/app/store/agent-pane-state/reducer.ts`
- agent-document reducer: `frontend/app/store/agent-document/`
- Phase-1 agent state machine refinements: PR #752
- State module: `frontend/app/view/agent/state.ts` (read/render projection)
- Stream consumer: `frontend/app/view/agent/useAgentStream.ts:87` (dispatch-only contract)
