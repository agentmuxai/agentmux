# What's next (2026-05-01)

**Context:** Phase E.5 + F1.A + F1.B shipped; smoke confirmed state-correctness; drag UX is governed by the Chrome-faithful tear-off spec, Phases 2-7 unstarted.

This is the forward plan. Read `docs/retro/phase-e-status-2026-05-01.md` for the consolidated status; this doc is just *what to work on next*.

---

## 1. Top priority — Chrome-faithful tear-off, Phases 2-7

**Spec:** `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`
**Already shipped:** Phase 1 (threshold detection) in PR #559.
**Why this is #1:** the original requirement was Chrome-style window-follows-cursor tear-off. We side-tracked into reducer/saga state-correctness because the existing tear-off pipeline made it impossible to fix the smoke regression without first fixing the underlying state divergence. Now state-correctness is solid; coming back to the drag UX is what makes the feature feel right.

**Phases (from the spec):**

| Phase | Scope | Notes |
|---|---|---|
| 1 | Threshold detection — drag distance crossing kicks the tear-off | ✅ #559 |
| 2 | Win32 `SC_MOVE` handshake — window detaches and follows cursor at threshold-cross | ⛔ |
| 3 | Pre-warmed window pool (0 ms first-paint flash) | ⛔ pool exists; Phase 3 is the *full* integration the spec describes |
| 4 | Cross-window merge — drop on another window's tab strip | ⛔ — replaces today's `cross-drag-update`/`cross-drag-end` plumbing |
| 5 | Cancel-back to source — ESC or drop-on-source-strip restores at original index | ⛔ — uses existing `RestoreTornOffTab` saga |
| 6 | macOS / Linux parity | ⛔ |
| 7 | Wayland fallback (dragend pipeline kept; Wayland forbids global cursor tracking) | ⛔ |

**State-layer hookups (already in place):** Phases 4-5 dispatch `TearOffTabSaga` / `TearOffBlockSaga` / `RestoreTornOffTabSaga` / `MoveTabToWorkspace` against the reducer. No new state work needed; the spec's UX rewrite plugs into the existing endpoints.

**Risk factor:** the spec's Quality Bar (§0) is strict — "no MVP cuts," "no fallbacks," sub-8ms handshake. Treat as a multi-PR effort.

---

## 2. Parallel-trackable Phase E sub-phases

These can run alongside the tear-off rewrite — no shared files, no design dependency.

### 2.1 E.2c.5b — TypeScript renderer dispatcher (cheap, unblocks E.6)

**Scope:** Install `window.__agentmux_srv_event` handler in the renderer; route the events the host already forwards into atom domains.

**Estimate:** ~150 LOC TS, single PR.

**Why now:** the host bridge has been forwarding events since #618 — they're currently being received but not consumed. Implementing the dispatcher unblocks E.6 (renderer multi-source + saga buffering) and gives the renderer a cleaner event-driven story than the current per-bespoke-channel pattern.

**Spec:** `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.6.

### 2.2 E.4 — Layout state migration (medium, has design questions)

**Scope:** Reducer commands for `LayoutState` mutations: focused/magnified node, `pendingbackendactions`, `leaforder`, `rootnode`.

**Open design question — granularity:**
- "Set rootnode" is too coarse (every drag-resize fires).
- "Patch node N's size" needs a node-path representation.
- The minimal slice (focused/magnified only) is straightforward; the full layout migration is its own phase.

**Estimate:** ~500-1000 LOC depending on granularity decision.

**Why this matters:** the only remaining wcore-direct write paths in `service.rs` are the SQLite-first delete patterns (DeleteBlock / DeleteWorkspace / CloseTab — intentional) **plus** the layout writes done inline by the TearOffBlock + PromoteBlockToTab RPC handlers (`setup_torn_off_block_layout`, `queue_source_layout_delete`). Those go through wcore because layout isn't reducer-routed yet.

**Defer until** the granularity question is decided. Not blocking anything.

### 2.3 E.6 — Renderer multi-source + saga buffering

**Depends on:** E.2c.5b.

**Scope:**
- Per-source version tracking (`launcher_event_version` vs `srv_event_version`) so dropped events are detectable.
- Saga buffering: when `SagaStarted { saga_id }` arrives, buffer subsequent saga_id-tagged events; flush atomically on `SagaCompleted`.
- Resync ordering: snapshot replay must precede live-event application.

**Estimate:** ~400 LOC TS.

**Saga_id correlation infra is already in place** from E.5 (lifecycle events emit saga_id). Per-Command/Event saga_id threading was deferred — E.6 lands it if buffering proves necessary.

### 2.4 E.7 — Phase exit (property tests + diag tools)

**Scope:**
- Property tests: reducer arm invariants (cascade integrity, idempotency, version monotonicity).
- Integration tests: real coordinator + reducer + subscriber + in-memory wstore. End-to-end saga happy paths + crash-mid-saga recovery.
- Diag tools: `--diag srv` (state-now), `--diag sagas` (in-flight registry). Mirror the launcher's `--diag wrr` pattern.

**Estimate:** ~600 LOC.

**When:** Phase E exit. After tear-off spec lands + E.4/E.6 ship, run E.7 to lock in the invariants.

---

## 3. Phase F+ (no spec — design when scheduled)

These remain on the parking lot; they won't fit cleanly until someone writes the spec.

| Item | Triggered by | Estimated effort |
|---|---|---|
| Cross-process saga (host pool-promote in saga) | Tear-off spec Phases 2-3 may surface this | 800-1200 LOC, multi-PR |
| Renderer registration as saga step | Tear-off spec Phase 2 | 300 LOC |
| Persistent saga state across srv restart | Probably waits for Phase G's event-sourced model | 1000+ LOC |
| Phase F host reducer | Long-term | Most of host's `browsers` / pool maps "resist the reducer pattern" per `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13 |
| Phase G event-sourced (drop SQLite) | Optional architecture refactor | Spec is sketched in `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §14 |

---

## 4. Don't-do list

- **Don't fix the empty `cross-drag-end` payload bug** in `agentmux-cef/src/commands/drag.rs::cancel_cross_drag`. The whole `start_cross_drag` / `update_cross_drag` / `complete_cross_drag` triad gets replaced when tear-off spec Phases 2-4 land.
- **Don't add more sagas to PR-on-PR for E.5 follow-ups.** Phase E.5 is closed. Anything new is either part of the tear-off spec or a fresh sub-phase with its own scope doc.
- **Don't reinstate strict validation in the reducer's `MoveTab` arm yet.** It's intentionally migration-tolerant (lazy-imports unknown tabs, drops workspace_id check) since some bootstrap edge cases can still produce stale state.tabs. Tighten in PR 4 cleanup AFTER tear-off spec ships and we've smoked the new flow extensively.
- **Don't smoke the drag UX again** before tear-off spec Phases 2-7 land. The half-hybrid is known-broken in expected ways; confirming that won't surface anything new.

---

## 5. Suggested ordering

If picking one thing to start tomorrow:

1. **Tear-off spec Phase 2 (SC_MOVE handshake-from-threshold-cross).** Biggest user-visible win; reducer foundation is ready to receive its calls.

If looking for a quick warm-up before tackling Phase 2:

2. **E.2c.5b (renderer dispatcher).** Small, self-contained, unblocks E.6, gets the renderer into a cleaner shape that the upcoming buffering work can build on. Could ship in a single short PR.

If wanting to defer drag work entirely and chip at the state migration:

3. **E.4 minimal-slice (focused/magnified node).** Decide granularity, ship the minimum viable layout reducer, leave `rootnode` / `leaforder` / `pendingbackendactions` for follow-ups. Lets us start retiring the wcore-direct layout writes in TearOffBlock / PromoteBlockToTab.

---

## 6. Cross-references

- `docs/retro/phase-e-status-2026-05-01.md` — full status doc; §11 has the smoke result + drag-UX deferral; this doc focuses on "what to do next."
- `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md` — the tear-off spec (#1 priority above).
- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — original Phase E spec; §13 sketches Phase F, §14 sketches Phase G.
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — saga decisions + robustness gaps; read before resuming Phase F+ work.
- `docs/retro/next-steps-2026-04-29.md` — older "what's next" doc; superseded by this one.
