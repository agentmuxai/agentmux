# REPORT — Agent Pane Architecture: Full Inventory & Modularization Analysis

**Date:** 2026-07-31
**Type:** Architecture analysis (investigation only — no code changed by this report)
**Trigger:** "Do an analysis of the agent pane, all the things that plug into it, should we do modularization/cleanup?"
**Related:** `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` (this session's earlier, narrower finding — the stuck-"Working" bug and the "everything leaks" observation that prompted this deeper look), `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` (**pre-existing**, independently-written report that already diagnosed the backend half of this exact problem 9 days before this investigation), `docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07-21.md` (the precedent fix — `CredentialBroker` — this report's recommendation follows the same playbook).

---

## 0. Headline finding, up front

**The single most important thing this analysis found is not a new problem — it's that this problem is already diagnosed, already has a design, and is already half-built.** `agentmux-srv/src/broker/process.rs` ("Process Broker — Phase A") exists in this tree right now, its own module doc stating AgentMux answers "is this agent/process alive and what's it doing" **six different, only-partially-overlapping ways**, modeled explicitly on a `CredentialBroker` that already fixed an analogous three-way duplication in auth. Phase A (read-side consolidation) shipped 9 days before this investigation (2026-07-22). Phase B (write-side — each `Controller` impl registering with the broker directly, closing the coverage gap instead of papering over it) is **explicitly deferred, not started**.

Everything this session found independently — the `task dev` idle-kill false positive, the Activity Dock's invisibility to detached processes, the orphaned `ToolNode` bug, the stuck-"Working" pane bug, and everything below — is a instance of the *same* root cause that report already named. The recommendation in §7 is not "build a new unifying state machine" — it's **"finish the state machine that's already being built, instead of starting a seventh mechanism."**

---

## 1. Scale — what "the Agent pane" actually is

| Layer | Location | Size |
|---|---|---|
| View (components, hooks, flows, providers, styles) | `frontend/app/view/agent/` | **253 files, ~61,700 lines** |
| State/reducers directly backing the view | `frontend/app/store/agent-pane-state/`, `agent-document/`, `agent-pane-layout/`, plus in-tree state machines (`auth-state.ts`, `settled-grace.ts`, etc.) | **~6,787 lines across 12 distinct reducer/state-machine-shaped modules** |
| Backend (Rust) — controllers, health, process tracking | `agentmux-srv/src/backend/blockcontroller/` + `process_tracker/` + `subagent_watcher/` | **~18,500 lines**, concentrated in one file (`persistent.rs`, 4,888 lines — over a quarter of `blockcontroller/` alone) |
| **Total** | | **~87,000 lines** directly supporting one pane type |

For comparison: the backend's own architecture report independently measured a **6×** duplication of "is this thing alive" logic; this session's frontend investigation independently found a comparable pattern one layer up (§4). Two teams/sessions, working nine days apart with no shared context, converged on the same diagnosis from opposite ends of the stack — that convergence is itself signal, not coincidence.

---

## 2. View layer (`frontend/app/view/agent/`) — structural findings

Full per-file inventory available in this investigation's raw output (not reproduced here in full — see the file list below for the parts that matter for a modularization decision).

### 2.1 The root component is doing five jobs in one file

`agent-view.tsx` — **1,959 lines**, two exported components (`AgentViewWrapper` at 143-159, `AgentPresentationView` at 164-1959). The latter interleaves, with no internal sub-component boundaries, in one shared closure scope:

| Concern | Approx. lines | Notes |
|---|---|---|
| Fork tab strip (switch/rename/**async create** with its own RPC round-trip) | ~140 | Self-contained sub-feature, fully embeddable |
| Pane registration/dispose diagnostics | ~85 | Includes a render-trail dump on mid-turn unmount |
| **Turn-lifecycle / controller-status reconciliation** | **~320** | The single largest block — `reconcileTurnActive`, `trackTurnJustEnded`, `postSystemNotification`, the settled-grace effect (source of the "Picked up more work…" bug from the prior investigation this session), focus/visibility re-poll |
| Command handling / message-send orchestration | ~180 | `handleSendMessage`, `retryLastTurn`, failure-recovery callback wiring (6 callbacks) |
| Held-message flush + async startup-sequence assembly | ~110 | RPC gathering for the first-turn payload |
| Search/keyboard/zoom/file-drop/context-menu wiring | ~80 | |
| **JSX render tree** | **~420** | |

This is not a new observation about this codebase's general style — the same file *already* shows the correct instinct applied inconsistently: `flows/launch-flow.ts` (361 lines), `fork/fork-set.ts` + `fork/useForkSet.ts`, and `activity/*-adapter.ts` were all **already extracted** from what was presumably a larger `agent-view.tsx` at some point. The 320-line turn-lifecycle block and the 140-line fork-create flow are the two largest remaining concerns that haven't had the same treatment yet, and they're the most mechanically similar to what's already been split out.

### 2.2 Naming overlaps (cosmetic — none of these are bugs by themselves, but they cost a reader time)

| Pattern | Instances |
|---|---|
| "Login/auth flow" entry points, each asserting it's *the* sanctioned one | `flows/force-login.ts`, `flows/run-provider-login.ts`, `flows/seed-global-login.ts`, `flows/open-oauth-pane.ts`, `flows/register-seeded-account.ts`, `auth/auth-flow-controller.ts`, `commands/global/login.ts` — **7 files** |
| "…-model.ts" (four different *view-models*, one unrelated *catalog overlay*) | `agent-model.ts`, `agent-mcp-model.ts`, `agent-native-memory-model.ts`, `agent-skill-model.ts`, + `providers/model-overlay.ts` (LLM model catalog — different "model" entirely) |
| "Catalog" (four unrelated catalogs) | `defaults/cli-catalog.ts`, `providers/catalog.ts`, `providers/toolchain-catalog.ts`, `providers/widget-catalog.ts` |
| "State" (three separate state modules, same basename pattern, different directories) | `view/agent/state.ts`, `view/agent/auth/auth-state.ts`, `view/agent/virtualization/state.ts` |
| Adapters converging on one consumer | `activity/shell-adapter.ts`, `activity/tool-adapter.ts`, `activity/subagent-adapter.ts` → all feed `components/ActivityDock.tsx` (this one is a *fine* pattern, just noted for completeness) |

None of these cause bugs — they cost a newcomer time ("which of the 7 login files do I touch"), not correctness. Lowest priority in §7.

### 2.3 Provider/translator layer is well-factored already

9 CLI providers (`claude`, `muxcode`, `codex`, `gemini`, `qwen`, `kimi`, `openclaw`, `copilot`, `pi`), backed by only 5 translator implementations via a shared-format `switch` in `translator-factory.ts` (3 formats each reused by 2-3 providers). This is the *opposite* of a modularization problem — it's a small, clean N:M mapping. No action needed here.

---

## 3. Store/reducer layer — structural findings

12 distinct reducer/state-machine-shaped modules total ~6,787 lines (table in the raw investigation). The two that matter most for this pane:

- **`agent-pane-state/reducer.ts`** (1,118 lines) — the `TurnPhase` machine (`Idle | Submitting | Streaming | Interrupting | Done | Disconnected`), documented in its own header as "single source of truth for the turn."
- **`agent-document/reducer.ts`** (741 lines) — the conversation document (`DocumentNode[]`), including `scrubOrphanedInProgress` (lines 53-148).

**Confirmed structural overlap** (not a hypothesis — read directly from both files):

- `TurnPhase.Streaming.toolsActive` (a count) and `ToolNode.status` (a per-node enum) are both incremented/decremented by the **same** upstream `ToolStart`/`ToolEnd` events, independently, in two different reducers, with **no shared invariant check** that they agree.
- `agent-document/reducer.ts`'s `scrubOrphanedInProgress` and `agent-pane-state/reducer.ts`'s `StreamWatchdogTick`/liveness-recovery are two **independently-implemented** "the stream ended without a graceful signal, force-settle stale in-progress state" mechanisms — one operating on document nodes, one on the pane phase, solving the identical underlying problem (orphaned "in-progress" state surviving a stream drop) with zero shared code.
- This split is *deliberate* per the reducer's own comment ("intentionally separate per the conventions doc §11, 'no god-reducer'") — the finding here isn't "these should be merged," it's "two independent recovery mechanisms exist for the same root cause (stream drop / silent rejection) with no shared logic, so a fix to one doesn't imply a fix to the other" — exactly what happened this session (the orphaned-`ToolNode` bug and the stuck-`TurnPhase` bug are the *same underlying event class* — a Bash tool call that never resolves — manifesting as two separately-diagnosed, separately-fixed-or-not bugs).

---

## 4. Backend — structural findings

`blockcontroller/` (13,586 lines) + `subagent_watcher/` (4,223) + `process_tracker/` (695) ≈ 18,500 lines. `persistent.rs` alone is 4,888 lines — larger than the entire `process_tracker/` + `subagent_watcher/` combined, and the largest single file in the whole backend subsystem by a wide margin. It has already had one, recent, successful extraction: `persistent_resume.rs` (1,012 lines), pulled out this exact session (PR #2371/#2373) specifically because 3 directly-mutated fields raced across 4 independently-scheduled tokio tasks and caused a real, twice-recurring bug (issue #2368). That extraction is the backend analogue of the same "pull the racy/overloaded concern into its own pure module" move recommended for the frontend in §2.1 and §7 — same shape of fix, same file, same session, independently motivated.

**The `proc_status`/`turn_active` finding, confirmed:** `PersistentInner` carries both `proc_status: String` and `turn_active: bool` (the latter sourced from `HealthMonitor`'s independent output-silence-timer heuristic — a third signal), both folded into one wire snapshot (`BlockControllerRuntimeStatus`), which the frontend then collapses *again* into `TurnPhase`. That's **four representations of one question** — backend status string, backend turn_active bool, backend HealthMonitor's own silence timer, frontend TurnPhase enum — before you even get to the dock/registry mechanisms in §0. This exact redundancy is called out by name as reconciliation source #5 in the pre-existing Process Broker report.

---

## 5. Direct thread from this session's own bugs to this analysis

Every distinct issue chased this session maps onto one of the (now six-plus-one, counting `TurnPhase`) mechanisms the pre-existing report already named:

| This session's bug | Mechanism (per §0/§4's numbered list) |
|---|---|
| `task dev` killed by bashwrap idle-timeout | Not one of the six — a seventh, PTY-byte-level mechanism, external to all of them, with its own independent "is it still going" heuristic |
| `task dev` invisible in the Activity Dock when detached | Dock = conversation-transcript-derived (mechanism outside the six-item backend list entirely — a *frontend*-side seventh/eighth mechanism) vs. `process_tracker` (#2) — confirmed structurally separate, "none of them talk to each other" |
| Orphaned `sleep 45` `ToolNode` stuck "running" | Dock's own no-independent-liveness-check gap (§3) |
| Stuck "Working" pane / "Picked up more work" | `TurnPhase` reducer's session_end heuristic + 3-minute silence watchdog interacting badly — itself layered on top of backend mechanism #5 (`HealthMonitor`) |
| `task dev` background-task bookkeeping lost across a session restart | Outside this repo (harness-side), but same *shape*: an untracked ninth "is it still running" approximation |

**None of these needed a new mechanism to fix. Every one of them is a symptom of there being too many un-unified mechanisms already.** Adding a state machine *for the frontend* without connecting it to the backend's already-in-flight Process Broker would create an eighth/ninth mechanism, not fewer.

---

## 6. What's genuinely fine as-is (don't touch)

- The provider/translator N:M mapping (§2.3) — small, clean, not a duplication problem.
- The three `activity/*-adapter.ts` files feeding one dock — a legitimate adapter pattern, not sprawl.
- The intentional separation between `agent-pane-state` and `agent-document` reducers ("no god-reducer") — the *split* is correct; the gap is the lack of a shared recovery invariant between their two independent orphan-handling paths, not the split itself.
- `persistent.rs` at 4,888 lines is large but is a single `impl Controller` trait body for the most complex controller type — matches the same constraint noted for `shell/lifecycle.rs` ("kept as one file because a trait impl can't be split across modules"). Not a target for splitting on size alone; only extract *sub-concerns* that are pure/independently-testable, as `persistent_resume.rs` already did once.

---

## 7. Recommendation — prioritized, incremental, no big-bang rewrite

In order of value-for-risk, matching how this codebase already does these extractions (small, one-concern-at-a-time, following an established local pattern rather than a from-scratch redesign):

1. ~~Implement `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md`.~~ **Done — already merged, not actually pending.** Corrected 2026-07-31: this item's own "fully designed, awaiting go-ahead" framing (both here and in the spec it references) was stale. The fix landed via PR #2369 (merged 2026-07-30T14:18:18Z, `dcf730956`), independently of this report — confirmed by reading `claude-translator.ts` directly rather than trusting the spec's self-reported status. It postdates `v0.54.7` (cut 2026-07-29), so it isn't in the currently-running production build yet; no further code action is needed here, only confirming a future release picks it up. See `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` §5.4 for the full account. **Lesson: a spec's own "Status:" header is not ground truth — check `git log`/PR state before recommending "implement this now."**
2. **[Now the top open item] Resume Process Broker Phase B** (`agentmux-srv/src/broker/process.rs`'s own deferred scope: each `Controller` impl registers with the broker directly at spawn, closing the coverage gap at the write side instead of the read side). This is the existing, half-finished initiative that already subsumes most of what a from-scratch "state machine for all these things" would try to build. Migrating `PersistentSubprocessController`/`ShellController`/`AcpController`/`SubprocessController` to register with it directly would let `proc_status`/`turn_active`/`HealthMonitor` collapse into the broker's own `ProcessStatus`, removing 3 of the 4 backend-side representations named in §4.
3. **Add the still-missing "long-running attached process" `TurnPhase` variant** (already recommended in this session's earlier spec, from `retro-persistent-agent-working-status-stuck-2026-07-16.md`'s own "Fix direction") — this is the frontend-side analogue of item 2, and should probably be designed *together* with it once the Process Broker's write side exists, so the frontend has one real signal to read instead of reconciling `controllerstatus` + dock state + registry polling itself.
4. **Split `agent-view.tsx`'s two largest remaining un-extracted concerns**, following the exact pattern already used for `flows/`, `fork/`, `activity/`:
   - Turn-lifecycle/controller-status reconciliation (~320 lines) → a `turn-lifecycle/` or extend `hooks/useTurnLifecycle.ts` (which already exists at 242 lines and is a natural home).
   - Fork-tab-strip event handlers, including the async create flow (~140 lines) → already has a home ready-made (`fork/`), just needs the handler logic moved alongside `fork-set.ts`.
   - This is refactor-only (no behavior change), moderate risk (large file, needs careful review), but directly reduces the file that's 5.5× the next-largest component in the whole view tree.
5. **Give the dock/document layer a shared orphan-recovery invariant** between `scrubOrphanedInProgress` and the pane's own liveness watchdog (§3) — smaller, more contained than items 2-4, but closes the exact bug class (orphaned in-progress state) hit twice this session in two different forms.
6. **Lowest priority — naming cleanup** (§2.2): rename for clarity (e.g. `providers/model-overlay.ts` → `providers/llm-model-overlay.ts`, consolidate the 7 login-flow files' doc comments to cross-reference each other explicitly). Cosmetic; do opportunistically, not as a dedicated pass.

**Explicitly not recommended:** a single big consolidation PR touching frontend and backend together, or introducing a brand-new cross-cutting state-machine abstraction that doesn't build on the Process Broker already in flight. The existing incremental pattern (extract one concern, verify, ship) is working — `persistent_resume.rs` (this session) and the Process Broker's own Phase A (9 days ago) are both examples of it succeeding. Item 2 above is the single highest-leverage piece of work here, precisely because it's already designed and partially built — finishing it, rather than re-diagnosing the same problem a third time, is the actual fix to "why does everything keep leaking."

---

*Investigation only. No files were changed as part of producing this report. Awaiting direction on which of §7's items (if any) to pursue.*
