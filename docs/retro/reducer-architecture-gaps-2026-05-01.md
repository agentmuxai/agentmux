# Reducer Architecture — Remaining Gaps

**Date:** 2026-05-01
**Author:** AgentA
**Status:** Snapshot — companion to `phase-e-status-2026-05-01.md` and `next-steps-2026-05-01.md`

---

## Why this doc exists

After Phase E.5 / E.7 closeout we asked: *"What gaps will remain in the reducer system before we proceed with exotic features and fixes?"*

This is the honest answer. It catalogs everything the multi-reducer architecture **does not yet cover**, even after E.5, E.7, and the Phase F spec land. It is the punch list that needs to drain before we declare the architecture *complete* — not just the next iteration of features.

The reducers we are talking about:

- **Launcher reducer** — Phase B, shipped. Drives launcher → host lifecycle.
- **Srv reducer** — Phase E, shipped. Drives backend mutations of workspaces / tabs / blocks.
- **Host reducer** — Phase F, **spec only** as of `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`. No implementation.

The compromise summary up front:

> Most-critical state correctness is in. Architectural completeness is **~60%**. The remaining 40% is what this doc enumerates.

---

## 1. Phase E sub-phases still open

E shipped through E.5 + E.7. The other branches of the Phase E tree are still open.

### E.4 — Layout reducer migration
Layout mutations (split / resize / move within a tab) still flow through `wcore` direct paths, not through the reducer. They predate Phase E and were intentionally deferred.

**Why it matters:** layout edits race tab-level edits during multi-pane flows (e.g. tear-off-while-resizing). Today this is rare enough to be invisible; it becomes load-bearing once host reducer (F) drives layout from CEF events.

### E.6 — Renderer multi-source + saga buffering
The renderer dispatcher (`frontend/util/srv-events.ts`) shipped in E.2c.5b as **scaffolding only** — it consumes one source. Multi-source consumption (srv + host events arriving on separate pipes, ordered + buffered while a saga is in flight) is not implemented.

**Why it matters:** until E.6, frontends can only react to *one* reducer at a time. Cross-reducer flows (host emits BrowserClosed → srv emits BlockDeleted → frontend reflects both) are not coherent.

### E.7 — Integration tests
We have proptest coverage of reducer arm invariants (`invariants_hold_across_random_sequences`, `delete_workspace_cascades_cleanly`) and the `--diag srv` operator command. We do **not** have:
- End-to-end saga tests (drive a saga through srv + host stubs, assert final state)
- Cross-pipe ordering tests
- Recovery-from-crash tests (saga partway through, reload, resume or roll back)

---

## 2. Phase F (host reducer) — spec only

F1.A (persist subscriber) and F1.B (orphan cleanup) shipped in srv as preparation for F. The host reducer itself is **not implemented**. Spec lives at `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`.

Even after F1 ships, these remain permanent gaps unless we explicitly close them:

### CEF browser handles
The `browsers` HashMap is owned by the host process (`agentmux-cef`). It is not reducer-managed today and the spec does not propose moving it. The "snapshot-and-drop" pattern keeps FFI safety but means browsers are not first-class state — they cannot be queried, replayed, or reasoned about by sagas without round-tripping through host commands.

### Warm pool
The CEF warm pool (pre-allocated browsers waiting for promotion) is host-private state. Sagas cannot ask "is there a warm browser available?" without an extra command. This is a design choice, not a bug, but it limits saga sophistication (e.g. *"if pool has a browser, use it; else create-and-wait"* is currently host-side branching).

### BlockController
`BlockController` instances (terminals, agents, etc.) are spawned by the host and tracked in a separate registry from the reducer. State transitions (start → running → exited) flow through events but are not authoritative reducer state.

---

## 3. Robustness gaps surviving every Phase

Some gaps are independent of which phase you are in — they are characteristics of the *current* saga design and they are not on any phase's checklist.

### Host pool-promote inside saga
When a saga needs a browser (e.g. RestoreTornOffTab), it currently calls into the host *outside* the saga's transactional envelope. If the host fails to promote a browser the saga rolls back srv state but the partial host action is not undone. Today this is benign (host action is "look at the pool") but becomes load-bearing if pool state ever mutates pre-promotion.

### Renderer registration as a saga step
When a new tab opens, the frontend registers a renderer with srv via a side channel — not as a saga step. If the saga commits but the renderer registration fails, srv has a tab the frontend doesn't know about. Recovery requires user resync.

### Saga state durability
Sagas live in memory. If the srv process crashes mid-saga, the partial state is lost. Recovery on next boot is "best effort via SQLite reconciliation" — there is no durable saga log.

**Why it matters:** this is fine for the saga set we have today (tear-off, restore, promote). It blocks adding sagas that span seconds-to-minutes (e.g. "spawn a remote agent, wait for it to register, attach it to a tab").

### Per-Command / per-Event saga_id threading
We added `saga_id` to lifecycle events. We did **not** thread it through every Command and every Event. So a debug tool can answer *"which saga was running when this lifecycle event fired"* but cannot answer *"which saga produced this BlockMoved command"*.

---

## 4. Reducer-pattern compromises shipped

These are places where the reducer is *not* the source of truth, even after E.5. They were intentional pragmatic compromises during migration; they are still compromises.

### `handle_move_tab` migration tolerance
`handle_move_tab` lazy-imports unknown tabs from SQLite and skips the workspace_id check. This was needed to unblock the cross-window drag fix (PR #621 codex P1 round-2). It means: *the reducer trusts SQLite for tabs it doesn't know about*. Real reducers should reject unknown commands.

**How long it stays:** until E.4 (layout migration) closes — at that point the reducer should know every tab from boot.

### Three SQLite-first deletes
`DeleteBlock`, `DeleteWorkspace`, and `DeleteTab` apply to SQLite first, then emit reducer commands. This predates E.5 and the saga coordinator. They short-circuit the reducer pattern (state mutated outside reducer; reducer is informed after the fact).

**How long they stay:** until we redesign cascade deletes as proper sagas. This is *not* on any current phase's checklist.

### `merge_meta_patch` pass-through
Meta updates (`SetMeta`, `SetMetaWildcard`) flow through the reducer but are largely opaque — the reducer doesn't validate the patches, just merges. So invariants on meta fields (e.g. "agent block always has `agent_id` set") are not enforced.

---

## 5. Cross-pipe coordination unvalidated

The architecture assumes srv and host emit events on separate pipes that the renderer consumes in order. Today:

### Resync ordering
On resync, srv sends a snapshot. Host sends its own snapshot independently. If they arrive interleaved with a live event stream, the renderer's ordering is best-effort.

### Per-source version tracking
Each reducer has a version counter. The renderer is supposed to track a high-water mark per source and skip stale events. The plumbing exists in scaffolding (`srv-events.ts`) but is not exercised by tests or by a multi-source flow.

### Force-push behavior
When a renderer asks for a force-push (e.g. "I lost connection, send me everything"), the protocol does not specify how concurrent live events should be handled. Today: best effort.

---

## 6. Platform parity

### `--diag` is Windows-only
The `--diag srv` operator command and the launcher's `--diag wrr` rely on Windows-specific process inspection (Job Objects). macOS and Linux have stubs that no-op. This is fine for a development tool but means our debug story is platform-asymmetric.

### Wayland deferred
Wayland-specific window features (CSD vs SSD negotiation, multi-screen DPI tracking) are explicitly out of scope for Phase F.

---

## 7. Phase G (event-sourced reducer; drop SQLite)

Sketched only in older docs (`multi-reducer-proposal-2026-04-28.md`). The endgame is to make the reducer event-sourced — no SQLite, just an append-only log + materialized views. This would close most of §3 and §4 by construction.

We are not on track for this. There is no phase plan for it. It is the architectural ceiling, not a next step.

---

## What "complete" looks like

If we wanted to declare the reducer architecture *complete* (= no more reducer-system work for the foreseeable future), we would need to close:

- E.4, E.6, E.7 integration tests (Phase E remaining)
- F1, F2, F3 (Phase F implementation per spec)
- §3 robustness gaps (host pool-promote, renderer registration, saga durability, saga_id threading)
- §4 compromises (kill the SQLite-first deletes, lock down `merge_meta_patch`)
- §5 cross-pipe tests + force-push spec

That is roughly **6–10 weeks of focused work**, not counting follow-on bugs.

What we have today is *enough to ship features on top of without those features racing or corrupting state under normal load*. It is **not** enough to onboard a second team to without a tour guide.

---

## Recommended next-step ordering

If we want to drain this list (rather than ship features on top), the highest-leverage order is:

1. **F1** — host reducer minimum. Closes §2 partially, §3 host pool-promote, §5 cross-pipe.
2. **E.6** — renderer multi-source. Required for F1 to be visible end-to-end.
3. **E.4** — layout reducer migration. Closes §4 `handle_move_tab` tolerance.
4. **§3 saga durability** — durable saga log. Unblocks long-running sagas (remote agents).
5. **§4 SQLite-first deletes** — convert to sagas. Largest compromise still in production.
6. **F2, F3** — full host reducer per spec.
7. **E.7 integration tests** — once F1 + E.6 are landed, the integration surface is finally testable.

Phase G remains the long-term ceiling.

---

## Cross-references

- Phase E status: `phase-e-status-2026-05-01.md`
- Forward plan: `next-steps-2026-05-01.md`
- Phase F spec: `../specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`
- Saga coordinator location: `saga-coordinator-location-analysis-2026-04-30.md`
- Tear-off spec (downstream consumer): `../specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`
