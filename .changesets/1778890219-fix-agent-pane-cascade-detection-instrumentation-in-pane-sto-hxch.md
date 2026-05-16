---
type: patch
---

fix(agent-pane): cascade detection + dispatchIfRegistered migration

- Detect reactive cascades that dispose a pane mid-dispatch (`agent-pane-state-store.ts`, `agent-document-store.ts`); log a `CASCADE_DETECTED` warning identifying the projection setter that triggered the dispose.
- Add `dispatchIfRegistered` soft-variant on both pane stores; migrate 22 async-context call sites (RAF, setTimeout, setInterval, subscription handlers, RPC `.catch()` continuations) across `useAgentStream.ts`, `useAgentCommands.ts`, `useHistoryPagination.ts`, and `agent-view.tsx` so they silently no-op instead of throwing when the pane disposed mid-dispatch.
- Guard the `browser-model.reload()` RAF callback with `if (this.closed) return;` to match every other IPC handler in that file.

Throwing `dispatch()` stays as the contract for synchronous-body register-order checks. Backed by new `agent-pane-state-store.test.ts` covering both contracts (5 tests, 306 total still pass).
