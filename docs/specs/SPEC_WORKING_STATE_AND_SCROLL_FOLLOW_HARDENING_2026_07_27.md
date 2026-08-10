# SPEC: Harden the "Working…" indicator and message-list auto-follow against four related recurring bugs

**Date:** 2026-07-27
**Status:** implemented — this pass's mechanisms shipped; of §5's two deferred items, the tool-liveness gap was closed by the attached-task axis (PR #2489 + #2472) and the scroll input-gating race remains open, conditional on live reports. Verified 2026-08-10.
**Author:** Agent1
**Scope:** `agentmux-srv/src/backend/blockcontroller/{mod,persistent}.rs`, `frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/view/agent/hooks/usePendingMessageAcceptance.ts`, `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`

---

## 1. Why this spec exists

Four distinct-but-related live incidents surfaced in one session, each a variant of "the pane's status doesn't match what actually happened, with no explanation":

1. This agent's own pane got stuck showing "Working…" after a PR merge that had genuinely completed.
2. Research into (1) surfaced a cataloged, still-open architectural gap: the backend's turn-end event is fire-once with no replay, so a missed push has no self-heal path (`docs/specs/REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md` §3/§4 item 5).
3. The message list's auto-scroll-follow, previously "fixed" (`docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`), was still observed silently drifting off true bottom.
4. Live, mid-session: Agent2's pane got stuck on an unanswerable AskUserQuestion prompt, and separately, a pane was observed going "Worked" → "Working…" again with no user input in between (`docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md`).

Each was investigated independently (two parallel research agents for #1/#3, direct code-grounded analysis for #4's two issues). This spec folds the findings into one implementation pass, since three of the four root causes are small, independent, and low-risk enough to land together; the fourth (settled-grace) and the AskUserQuestion race are handled with more care (see §4, §5).

## 2. Mechanism 1-3: closing the stuck-"Working" gap (backend fire-once event)

Root cause (confirmed via code + the two prior reports): `publish_controller_status` (`agentmux-srv/src/backend/blockcontroller/mod.rs`), the sole publish point for `controllerstatus` across all 13 controller-type call sites, published with `persist: 0` — a WS reconnect (network blip, srv restart, backgrounded window's socket actually dropping) gets nothing until the *next* live event, so a missed terminal turn-end push leaves the pane stuck showing "Working" indefinitely.

Three independent, layered fixes (deliberately redundant — no single one has to be perfect):

**Mechanism 1 — persist the event (`mod.rs`).** Changed `persist: 0` → `persist: 1`, mirroring the exact precedent already in production for `EVENT_AGENT_FAILURE` (`subprocess/host_spawn.rs`). A reconnecting subscriber now replays the last known status atomically instead of seeing nothing. Regression test: `test_publish_controller_status_persists_for_replay` (publishes once, reads back via `Broker::read_event_history`).

**Mechanism 2 — periodic heartbeat (`persistent.rs`).** New `spawn_status_heartbeat` method, wired alongside every existing `core::spawn_health_watchdog` call site (process spawn, `send_message`, `send_user_message`). Republishes the current status every 20s while a turn is active, regardless of whether it changed — a self-healing backstop for the case a live push is missed for a reason *other* than a reconnect (e.g. a throttled/backgrounded renderer coalescing WS messages while the window stays connected). 20s is well below the frontend's own `STUCK_THRESHOLD_MS` (45s, diagnostic) and `LIVENESS_RECOVERY_MS` (180s, force-recovery), so a missed push self-heals long before either fires.

**Mechanism 3 — focus-triggered reconcile (`agent-view.tsx`).** New effect on `makeWindowFocusSignal()` (pre-existing, previously only consumed by sound notifications): on every background→foreground transition, calls `BlockService.GetControllerStatus` and reconciles, mirroring the existing mount-time one-shot. This is the one mechanism that doesn't depend on the WS connection's lifecycle at all — it covers the case a pane's subscription lifecycle (remount without a full reconnect) means persist-replay wouldn't refire, per the WPS broker's per-`(route_id, event_name, scope)` replay-once-per-connection semantics.

**Not done this pass (§5):** the frontend-side `toolsActive > 0` unconditional watchdog exemption (report 1's #1 gap — no positive liveness signal for a long-running tool call) needs an actual state-machine change and is sized as a separate follow-up. The `[wave-turn]` telemetry (report 1 §3.1/§3.2) turned out to already be implemented, landed via a concurrent PR (#2321) from another agent mid-session — verified present and working, no duplicate work done.

## 3. Mechanism 4: scroll-follow silently drifting off bottom

The 2026-07-24 fix (three tracked dependencies in the pin-to-bottom effect: `nodes().length`, `layoutView().totalSize`, `workingRowHeight`) was confirmed intact and unregressed. The still-recurring symptom has a different, more common root cause than anything that fix targeted: **a sibling row below the scroll region (retry bar, `AgentDecisionPanel`, `AgentQuestionPanel`, `PendingMessagesPanel`) appearing or growing mid-turn shrinks the flex-1 scroll container's `clientHeight` via pure CSS reflow.** No `scroll` event fires (it's a resize, not a scroll), and none of the three tracked dependencies change either — `stickToBottom` stays silently `true` while the view falls short of the new max scroll position. The 07-24 spec's own §3.3 flagged these exact rows but dismissed them as "rare/transient"; they're common in any normal tool-approval/question flow.

**Fix (`AgentDocumentVirtualList.tsx`):** generalized the existing `ResizeObserver` on the scroll container — previously gated to only re-pin on the hidden→visible (0→N) transition — to re-pin on *any* `clientHeight` change while `stickToBottom()` is true. Idempotent when already at true bottom; still respects a user who deliberately scrolled away. Closes the entire class of "something below the message list changed size" bugs generically, instead of requiring a new tracked-dependency addition every time a new interposing panel is added (the whack-a-mole pattern the 07-24 fix already had to extend once, 2→3 deps).

**Secondary fix (`usePendingMessageAcceptance.ts` → `useAgentStream.ts` → `agent-view.tsx`):** a new turn starting via the queue-drain path (a message queued while busy, later auto-accepted by the backend) dispatched `TurnStart` without ever re-engaging `stickToBottom` — only turns started via this pane's own composer did. Threaded a new `onTurnStartFromQueue` callback through to call the existing `jumpToBottom`/`scrollToBottomFn` (which both scrolls and re-engages stick), so "any new turn re-engages follow" holds regardless of which path started it.

**Deferred (§5):** the input-gated / programmatic-scroll-vs-user-scroll race (root cause #2 in the scroll research) — recommended as a follow-up only if the ResizeObserver fix above doesn't fully resolve live reports, since it's the more common repro and cheaper to ship first.

## 4. "Worked" reverting to "Working…" with no user input — settled-grace invariant

Root cause (`frontend/app/store/agent-pane-state/reducer.ts`'s `StreamFlushObserved` case): `Done.completed` → `Streaming` re-promotion on any live flush is **intentional**, not a bug — the CLI's `session_end` fires after every model round, not just true turn end, so a genuine multi-round tool continuation needs this path; removing it would leave the UI falsely showing "Worked" during real work, a worse failure than the reverse.

**Fix (`agent-view.tsx`, additive only — no reducer/`TurnPhase` changes):** a `SETTLE_GRACE_MS` (500ms) timer arms when `turnPhase` enters `Done.completed`. If no further activity arrives within the window, the episode is considered *settled*. If a `StreamFlushObserved` re-promotion to `Streaming` arrives **after** the episode settled, a visible system notification ("Picked up more work — starting another round…") is posted into the conversation instead of letting the indicator silently flip back with no explanation. An episode still within its grace window (the genuine same-breath multi-round case) re-promotes exactly as before, silently — no regression to the common case, only the "user already saw it settle" case gets disclosed.

500ms and the exact notification wording are implementation defaults, not confirmed product decisions — flagged as reversible/tunable if they don't feel right in practice.

## 5. Explicitly deferred (not this pass)

- **Frontend tool-liveness gap** (report 1's #1 finding: `toolsActive > 0` is an unconditional, unbounded watchdog exemption). Needs a real `TurnPhase`/reducer change plus a backend process-liveness signal threaded into `controllerstatus` — sized as its own follow-up spec, not bundled here to keep this pass's risk low.
- **Scroll input-gating** (race root cause #2) — only pursue if live reports continue after mechanism 4 above ships; needs telemetry to confirm it actually fires before investing in wheel/pointer/touch listener plumbing.
- **Agent2's stuck-AskUserQuestion panel** — root cause is a hypothesis (an optimistic client-side "answered" transition racing a document resync that rebuilds from a transcript where the tail question is still `awaiting_answer`), not yet live-confirmed. Needs direct inspection of a live instance's document/transcript state before sizing a fix. See `docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` §2.

## 6. Verification

- Backend: `cargo check -p agentmux-srv` clean; full suite 1675 passed / 0 failed / 4 ignored (was 1674 before this change — net +1 from the new persist-replay regression test).
- Frontend: `npx tsc --noEmit` clean; `npx vitest run app/view/agent app/store/agent-pane-state app/window` — 66 files / 931 tests passed, 0 regressions.
- No live UI verification performed this pass (see this session's prior documented hazard with direct DB writes to a live srv instance — avoided here entirely; all changes verified via automated tests only).
