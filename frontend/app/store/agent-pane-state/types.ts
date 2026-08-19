// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the agent-pane-state reducer (slice #4 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md).
 *
 * Bundles the per-pane atoms that have cohesive cross-atom invariants:
 *   streaming, sessionStats, currentTool, turnTokens, pendingMessages,
 *   plus init phase, turn phase, and stream-watchdog fields added in
 *   issue #728.
 *
 * PR G (turn-phase cleanup): the legacy `turnActive` / `stopping`
 * booleans and `streaming.active` boolean were dropped — every view
 * now reads `isWorking(state)` / `state.turnPhase.kind` /
 * `isDisconnected(state)` instead. The reducer-internal "is the
 * stream subscribed?" gate (previously `state.streaming.active`)
 * now uses `state.lastEventMs !== null` — both moved in lockstep
 * already (only `StreamSubscribe` / `StreamUnsubscribe` ever set
 * them), so the substitution is mechanical.
 *
 * The agent document (message list) lives in its own slice
 * (`agent-document/`); reducer scope is intentionally separate per the
 * conventions doc §11 ("no god-reducer").
 */

import type { PendingMessage } from "../../view/agent/state";
import type {
    SessionStats,
    StreamingState,
    TurnTokens,
} from "../../view/agent/types";

/**
 * Init lifecycle — covers the "history still loading" gap (issue #728 gap 1).
 *
 * Discriminated union. The pane starts in `InitPending`; the history
 * load resolves into either `InitReady` (success) or `InitFailed`
 * (error, with diagnostic reason). Transitions are one-way: once a
 * pane has reached a terminal state (`InitReady` / `InitFailed`),
 * `InitStart` is a no-op. Re-mount creates a new pane slot whose state
 * resets to `InitPending`.
 *
 * Each terminal variant carries the wall-clock ms it was entered. The
 * timestamp is intentionally part of the variant payload (not a
 * separate field) so a transition is one atomic write.
 */
export type InitPhase =
    | { kind: "InitPending" }
    | { kind: "InitReady"; at: number }
    | { kind: "InitFailed"; at: number; reason: string };

// TurnPhase discriminated union — single source of truth for the turn
// lifecycle (since PR G).
//
// SPEC: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5.
// Consumers project via `isWorking` / `isDisconnected` / `state.turnPhase.kind`.

/** Why the turn entered Interrupting. */
export type InterruptReason =
    /** User pressed Esc / clicked Stop. */
    | "user"
    /** Caller-initiated (auto-stop, watchdog, etc.). */
    | "system";

/** Why the turn finished (kind: "Done"). */
export type TurnOutcome =
    /** Backend emitted session_end normally. */
    | "completed"
    /** Stop was requested before completion (user or system). */
    | "stopped"
    /** Stream torn down mid-turn — no graceful end. */
    | "interrupted"
    /** A non-zero turn ended without backend output (RPC fail, etc.). */
    | "errored";

/** The working-kind we were in before the stream dropped. */
export type KindBeforeDisconnect =
    | "Submitting"
    | "Streaming"
    | "Interrupting";

/**
 * Why the stream dropped while a turn was in-flight (PR F).
 *
 *   stream-unsubscribed : the dispatch layer / hook fired a clean
 *                         `StreamUnsubscribe` (component unmount, PTY
 *                         exited, etc.) — the local end tore down.
 *   transport-error     : reserved for an in-flight transport failure
 *                         surface; the dispatcher decides which reason
 *                         to attach. Not yet wired by any caller — the
 *                         union is part of PR F's API contract so the
 *                         dispatcher can adopt it without a follow-up
 *                         type churn.
 */
export type DisconnectReason = "stream-unsubscribed" | "transport-error";

/**
 * Live "compaction in progress" state — set the instant the `PreCompact`
 * hook's `compaction_started` WPS event lands (Tier 1 of
 * `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`),
 * cleared when the matching `compact_boundary` frame (a real
 * `CompactionBoundary` command) arrives. `startedAt` drives the live
 * elapsed-time counter (Tier 2) shown near the pane status chip.
 */
export interface CompactionState {
    trigger: "manual" | "auto";
    startedAt: number;
}

/**
 * Live "at least one agent-declared long-running task is attached to this
 * pane" state — independent of `turnPhase`. A sibling axis, not a
 * `TurnPhase` variant: see docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md
 * §2 for why (mirrors `CompactionState`'s own precedent for a concurrent-
 * but-orthogonal concern that must survive independently of the turn
 * lifecycle). Sourced from the dock's aggregated `PinnedActivity` list
 * (shell/subagent/tool adapters), not raw OS process counts — see spec §4.
 *
 * `since` is the wall-clock ms the current unbroken "≥1 running" episode
 * began — not reset by a second task starting while one is already live.
 */
export interface AttachedTaskState {
    since: number;
}

/**
 * A classified backend failure (the `agentfailure` wave event's payload,
 * `AgentFailure` in `frontend/types/gotypes.d.ts`) currently surfaced for
 * this pane. Single source of truth for "is there an active failure" —
 * previously duplicated as a hook-local signal in `useAgentFailure.ts`,
 * with no path back into `turnPhase`. See
 * docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
 *
 * Deliberately does NOT carry the auto-retry countdown/budget or the
 * expanded/retrying view flags — those are pure presentation timing with
 * no correctness coupling to anything else in the reducer, and stay
 * hook-local in `useAgentFailure.ts` exactly as before.
 */
export interface PaneFailure {
    /** The classified failure, verbatim from the backend event. */
    data: AgentFailure;
    /** Wall-clock ms the failure landed. */
    at: number;
}

/**
 * Single source of truth for the turn lifecycle. Since PR G this is the
 * only place where "is the agent working", "is a stop in flight", and
 * "did the stream drop" are encoded — the legacy `turnActive` /
 * `stopping` / `streaming.active` booleans have been removed.
 *
 *   Idle          : no turn in flight.
 *   Submitting    : user pressed send; awaiting `StreamSubscribe` / first
 *                   stream event.
 *   Streaming     : stream is actively producing events.
 *   Interrupting  : stop requested; awaiting graceful TurnEnd or unsub.
 *   Done          : turn finished (with outcome).
 *   Disconnected  : stream dropped while a turn was in-flight; remembers
 *                   the kind we lost so the view can show a re-attach UI.
 */
export type TurnPhase =
    | { kind: "Idle" }
    | { kind: "Submitting"; submittedAt: number; pendingContent: string }
    | {
          kind: "Streaming";
          bufferSize: number;
          toolsActive: number;
          lastEventMs: number;
          /** Set when the provider is rate-limited and retrying. Cleared on next real activity. */
          waitingReason?: "rate_limited";
          /** Milliseconds until the next retry, from the provider's Retry-After header. */
          retryAfterMs?: number | null;
      }
    | {
          kind: "Interrupting";
          reason: InterruptReason;
          sigintSentAt: number;
      }
    | { kind: "Done"; outcome: TurnOutcome; finishedAt: number }
    | {
          /**
           * Stream dropped while a turn was in flight. PR F fleshes out
           * the variant payload:
           *   - `lastKind`     : the working-kind we lost (so the UI can
           *                      say "was streaming" vs "was submitting").
           *                      Carried since PR A.
           *   - `lastConnectedAt` : wall-clock ms at which the disconnect
           *                      command was processed (= `command.at`
           *                      from `StreamUnsubscribe`). Drives a
           *                      "disconnected 5s ago" age label and
           *                      keeps the variant audit-friendly.
           *   - `reason`       : a finite literal union (was `string` in
           *                      PR A) — see {@link DisconnectReason}.
           */
          kind: "Disconnected";
          lastKind: KindBeforeDisconnect;
          lastConnectedAt: number;
          reason: DisconnectReason;
      };

/**
 * The reducer's state. Each field maps 1:1 to a Solid signal that the
 * agent pane projects from. The reducer enforces invariants ACROSS
 * fields (e.g. a turn can't enter Submitting/Streaming while the
 * stream is unsubscribed — see the `TurnStart` arm).
 *
 * PR G cleanup: `turnActive` / `stopping` (top-level) and `active` (on
 * `streaming`) have all been removed. `state.turnPhase.kind` is the
 * sole working-state encoding; "is the stream subscribed?" is now
 * derived from `state.lastEventMs !== null` (cleared atomically with
 * the stream on `StreamSubscribe` / `StreamUnsubscribe`).
 */
export interface AgentPaneState {
    streaming: StreamingState;
    sessionStats: SessionStats | null;
    /**
     * Cumulative cost/tokens/duration across every completed turn in this
     * pane's lifetime — unlike `sessionStats` (which is replaced, not
     * accumulated, on each TurnEnd), this sums into the previous value.
     * Feeds the composer-strip "totals" display near the input. Cleared
     * only on TurnReset. See SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md.
     */
    sessionTotals: SessionStats | null;
    currentTool: string | null;
    /** First significant argument of the active tool (file path, command, etc.).
     *  Cleared alongside currentTool on ToolEnd / TurnEnd / TurnReset. */
    currentToolArg: string | null;
    turnTokens: TurnTokens | null;
    /**
     * True for the duration of a turn started specifically to send a
     * manual "/compact" (the composer's "Compact now" button, or a user
     * typing it) — set on `TurnStart` when `command.content === "/compact"`,
     * consumed and reset by `CompactionBoundary`. Exists because a manual
     * `CompactionBoundary.trigger` alone is NOT a safe signal that THIS
     * pane's currently-working turn is the synthetic one that action
     * opened: the reducer's own tests deliberately model a manual
     * boundary landing while an unrelated real turn is already streaming
     * (compaction_started/compact_boundary travel over independent
     * transports with no ordering guarantee against the primary NDJSON
     * stream — see the CompactionStarted race-guard tests). Only end the
     * turn when we know FOR THIS PANE that its own working turn is the
     * one the manual compact opened. codex P1 on PR #2659.
     */
    pendingCompactTurn: boolean;
    pending: PendingMessage[];
    /**
     * Init phase — `InitPending` until history fetch resolves (or fails).
     * Discriminated union; failure reason lives in the `InitFailed`
     * variant's `reason` payload — no separate `initError` field.
     */
    initPhase: InitPhase;
    /**
     * Wall-clock ms of the last observable stream activity (subscribe,
     * flush, tool, tokens). Drives the stuck-stream watchdog.
     *
     * Doubles as the "is the stream subscribed?" gate since PR G:
     * `lastEventMs !== null` ⇔ the pane has seen a `StreamSubscribe`
     * that no subsequent `StreamUnsubscribe` has cleared. Both fields
     * always moved in lockstep when the legacy `streaming.active`
     * boolean existed, so the substitution is exact.
     */
    lastEventMs: number | null;
    /**
     * Single-source-of-truth turn phase. PR A added it with dual-write
     * against the legacy `turnActive` / `stopping` / `streaming.active`
     * booleans; PR B migrated every view consumer onto it via
     * `isWorking(state)` / `isDisconnected(state)` /
     * `state.turnPhase.kind`; PR G removed the legacy fields.
     */
    turnPhase: TurnPhase;

    /**
     * Whether the composer details panel (the expandable section that
     * holds the activity log, session stats, permission/model/effort
     * dropdowns, and Archive/Export/Restore buttons) is open. Default
     * `false`. Persists across renders within a pane lifetime; resets
     * to `false` on pane unmount (because a new pane gets a fresh
     * state via `initialState()`). Not persisted to backend — this is
     * a per-session ephemeral preference, same contract today's
     * AgentControlBar uses.
     *
     * SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4. Previously
     * auto-collapsed on `TurnStart`; removed once the panel started
     * hosting a live interactive shell (AgentShellSubblock) that needs
     * to survive sending more messages, not close on every send.
     */
    detailsOpen: boolean;
    /**
     * Input-token count from the most recent message_start — the full
     * context fill sent to the model on that turn. Unlike `turnTokens`,
     * this field is NOT cleared at TurnEnd so the context-window bar
     * stays visible between turns showing the last known fill level.
     * Cleared only when the pane is created (initialState) or on an
     * explicit TurnReset (session wipe).
     */
    lastContextTokens: number | null;
    /**
     * Learned context-window size for the current model — seeded from the
     * resolved model id on the first TokensIn and upgraded if observed context
     * ever exceeds it (Sonnet-1M detection). NOT a per-provider constant.
     * Null until a recognised model is seen; the view falls back to the
     * provider's static window. See store/agent-pane-state/context-window.ts.
     */
    lastContextWindow: number | null;
    /** Resolved model id that produced `lastContextWindow` — used to re-seed the
     *  window when the user switches models mid-session (`/model`). */
    lastContextModel: string | null;
    /**
     * Active classified failure for this pane, or `null`. Set by
     * `FailureObserved` (which also force-ends a working turn — see the
     * command's reducer case), cleared by `FailureCleared` or implicitly
     * by the next `TurnStart` (a fresh turn always ends the failure
     * episode). See {@link PaneFailure}.
     */
    failure: PaneFailure | null;
    /** Live "compaction in progress" state, or null. See {@link CompactionState}. */
    compacting: CompactionState | null;
    /**
     * Live "attached long-running task" state, or null — see
     * {@link AttachedTaskState}. Deliberately NOT cleared by any `TurnPhase`
     * transition (TurnEnd/TurnReset/etc.): a task attached in one turn can
     * legitimately survive into later turns, so only its own two commands
     * (`AttachedTaskObserved`/`AttachedTaskCleared`) ever touch this field.
     */
    attachedTask: AttachedTaskState | null;
    /**
     * Wall-clock ms of the most recent REAL `CompactionBoundary` event
     * (backend-sourced, exact — not the ≥50%-drop heuristic below). The
     * `TokensIn` heuristic checks this before firing its own synthetic
     * `context-compacted` event, so a Claude session that just got a
     * real boundary doesn't also get a duplicate heuristic-sourced one
     * for the very next turn's token drop. Providers without a
     * structured signal (codex/gemini/copilot) never set this, so the
     * heuristic remains their only detection path. See
     * `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §4.3.
     */
    lastCompactionBoundaryAt: number | null;
}

export const initialState = (agentId: string): AgentPaneState => ({
    streaming: { agentId, bufferSize: 0, lastEventTime: 0 },
    sessionStats: null,
    sessionTotals: null,
    currentTool: null,
    currentToolArg: null,
    turnTokens: null,
    pendingCompactTurn: false,
    lastContextTokens: null,
    lastContextWindow: null,
    lastContextModel: null,
    pending: [],
    initPhase: { kind: "InitPending" },
    lastEventMs: null,
    turnPhase: { kind: "Idle" },
    detailsOpen: false,
    failure: null,
    compacting: null,
    attachedTask: null,
    lastCompactionBoundaryAt: null,
});

/** Selector — `true` iff the pane has finished its initial history load. */
export function isInitReady(state: AgentPaneState): boolean {
    return state.initPhase.kind === "InitReady";
}

/**
 * Selector — true while the agent is processing a user request.
 *
 * Returns true ⇔ `state.turnPhase.kind ∈ {Submitting, Streaming, Interrupting}`.
 * Idle / Done / Disconnected are all "not working".
 *
 * Spec: SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §7. Since PR G this
 * is the only "working" predicate — the legacy `turnActive || stopping`
 * combination has been removed.
 */
export function isWorking(state: AgentPaneState): boolean {
    return workingFromPhase(state.turnPhase);
}

/**
 * Phase-only variant of {@link isWorking}. The view layer projects
 * `turnPhase` through a dedicated signal (PR B); call sites that only
 * have the phase (not the whole state) use this helper. Same predicate.
 *
 * Returns true ⇔ `phase.kind ∈ {Submitting, Streaming, Interrupting}`.
 */
export function workingFromPhase(phase: TurnPhase): boolean {
    const k = phase.kind;
    return k === "Submitting" || k === "Streaming" || k === "Interrupting";
}

/**
 * Selector — `true` iff the pane is in the `Disconnected` phase. PR F:
 * drives the {@link AgentDisconnectedBanner} visibility, replacing any
 * ad-hoc "streaming.active === false but turn was active" checks. Pure
 * projection of `turnPhase.kind` — same SoT as `isWorking`.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §6.4.
 */
export function isDisconnected(state: AgentPaneState): boolean {
    return state.turnPhase.kind === "Disconnected";
}

export type AgentPaneCommand =
    /**
     * Caller signal: history fetch began. Idempotent — if the pane is
     * already in any terminal state (`InitReady` / `InitFailed`),
     * `InitStart` is a no-op (we don't reset back to pending).
     */
    | { type: "InitStart" }
    /** Caller signal: history fetch resolved successfully. */
    | { type: "InitReady"; at: number }
    /** Caller signal: history fetch failed; reason surfaced for diagnostics. */
    | { type: "InitFailed"; at: number; reason: string }

    /** Hook signal: subscription is up. */
    | { type: "StreamSubscribe"; at: number }
    /** Hook signal: subscription torn down. */
    | { type: "StreamUnsubscribe"; at: number }
    /**
     * RAF flush observed by useAgentStream — bumps bufferSize +
     * lastEventTime. No state change beyond those counters.
     */
    | { type: "StreamFlushObserved"; addedCount: number; at: number }
    /**
     * Periodic tick from useAgentStream's watchdog interval. Emits a
     * `stream-stuck` diagnostic past `STUCK_THRESHOLD_MS`, and — past the
     * longer `LIVENESS_RECOVERY_MS` — force-recovers a hung `Streaming` turn
     * (no tool active, not rate-limited) to `Idle`, clearing a stuck
     * "Working" that never received its terminal `session_end`. See
     * SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md.
     */
    | { type: "StreamWatchdogTick"; nowMs: number }

    /**
     * Reconciliation from `BlockControllerRuntimeStatus.turn_active`
     * (backend-verified, from the health monitor wired to the NDJSON
     * stream — see `agentmux-srv/src/backend/blockcontroller/health.rs`),
     * fetched via `GetControllerStatus` at mount AND dispatched on every
     * live `controllerstatus` WPS event thereafter (useControllerStatusEvents).
     * Bidirectional:
     *   - `active: true`  — promote the mount-default `Idle` to `Streaming`
     *     when the backend reports a turn already in flight. Corrects ONLY
     *     an `Idle` phase; never overrides a phase a real stream/user event
     *     already produced (TurnStart, StreamFlushObserved, etc. win if they
     *     got there first). #2005 Finding 1.
     *   - `active: false` — demote a stuck `Streaming` phase to `Idle`. The
     *     backend flips `turn_active` false only on the CLI's `result` event
     *     (genuine turn-end), so this is authoritative. Normally the frontend
     *     reaches `Done` on its own via `session_end`; this covers the case
     *     where it MISSED that event (pane backgrounded/unmounted across the
     *     transition, or a dropped WPS event) and would otherwise stay
     *     `Streaming` forever — the Agent1 stuck-"Working" / Agent2
     *     stuck-"Queued" class. Only `Streaming` is demoted: `Submitting` is
     *     covered by `SUBMIT_TIMEOUT` (and is where the send-race lives — a
     *     just-dispatched local `TurnStart` racing a backend that hasn't yet
     *     marked the turn active), `Interrupting` by `INTERRUPT_TIMEOUT`,
     *     `Done`/`Idle`/`Disconnected` are already correct.
     * See docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md.
     */
    | { type: "ReconcileTurnActive"; at: number; active: boolean }
    /**
     * Mount-time reconciliation: history replay found a `session_end` stats
     * payload from before this pane went live (the resumed conversation's
     * last turn — or, if resume silently failed, whatever short session
     * replaced it). Seeds `lastContextTokens` so the composer strip's
     * context-fill bar shows a real number immediately instead of sitting
     * blank until the first live `TokensIn`. No-ops if a live `TokensIn`
     * already arrived first (mirrors `ReconcileTurnActive`'s
     * only-if-still-default guard) — a historical snapshot must never
     * clobber real-time data. See
     * docs/plans/PLAN_PANE_REOPEN_SESSION_RESUME_AND_STATS_BAR_2026_07_10.md.
     */
    | { type: "ReconcileContextFromHistory"; tokens: number }
    /**
     * User pressed send — turn becomes active. Also clears stale
     * sessionStats from the previous turn.
     *
     * `content` — the literal text just submitted, threaded through into
     * `TurnPhase.Submitting.pendingContent` (optional so the ~90 existing
     * reducer/hook test call sites that construct this action without it
     * keep compiling — the reducer defaults to `""` when absent). Used by
     * `useAgentActivitySummary.ts` to derive the session-title Haiku call
     * without a separate FileStore read. See
     * docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
     */
    | { type: "TurnStart"; at: number; content?: string }
    /**
     * Stream produced session_end (or fallback timer fired). Final
     * stats merged with current turn-tokens. Clears currentTool,
     * turnTokens, and transitions the phase to Done (interrupting →
     * Done.stopped, otherwise Done.completed).
     */
    | {
          type: "TurnEnd";
          stats: SessionStats | null;
      }
    /**
     * Truncate path: the doc was reset (or whatever caused the local
     * stream-restart). Clears all per-turn state but does NOT touch
     * pending or streaming.
     */
    | { type: "TurnReset" }
    /**
     * Revert an OPTIMISTIC `TurnStart` when the turn never actually began —
     * the initiating send's own RPC call failed synchronously (no
     * controller registered, the identity spawn gate blocked it, a plain
     * network rejection). Phase-only: unlike `TurnReset`, this must NOT
     * touch `sessionStats`/`sessionTotals`/`lastContextTokens` — those
     * accumulate across a pane's whole lifetime, and a transient send
     * failure on an agent with prior completed turns must not wipe that
     * history (reagent/codex P2 on PR #2318 — `TurnReset` was reused here
     * first and incorrectly cleared them). See
     * `useAgentCommands.ts`'s `deliverToBackend`.
     */
    | { type: "TurnStartFailed" }

    // ── Tool ───────────────────────────────────────────────────────
    | { type: "ToolStart"; name: string; arg?: string }
    | { type: "ToolEnd" }

    | { type: "TokensIn"; input: number; model?: string }
    | { type: "TokensOut"; output: number }

    /** User pressed Esc / clicked Stop. */
    | { type: "RequestStop"; at: number }
    /** Stop RPC failed — bail on the stopping state. */
    | { type: "StopFailed" }
    /**
     * Interrupt timeout watchdog fired (caller-scheduled setTimeout — see
     * `ScheduleInterruptTimeout` event). Bounded `Interrupting → Done`
     * force-transition: if the agent never acks the SIGINT (backend died,
     * stream dropped, etc.) the pane was previously stuck in Interrupting
     * forever and the working animation would never settle. This command
     * is a no-op if the phase has already moved off Interrupting
     * (e.g. agent acked first, user already resumed). Spec §8.
     */
    | { type: "InterruptTimeoutElapsed"; at: number }
    /**
     * Submit timeout watchdog fired (caller-scheduled setTimeout — see
     * `schedule-submit-timeout` event). PR D — bounded
     * `Submitting → Done.errored` force-transition: if the backend never
     * acks a TurnStart with a chunk / token / tool event within
     * `SUBMIT_TIMEOUT_MS`, the pane was previously stuck in Submitting
     * forever (no `isWorking → false` signal) and the working animation
     * would never settle. No-op if the phase has already moved off
     * Submitting (e.g. promotion to Streaming via flush / bumpEvent,
     * user already stopped, stream dropped). Spec §8 / issue #728 gap 2.
     */
    | { type: "SubmitTimeoutElapsed"; at: number }

    | {
          type: "PendingMessageQueued";
          id: string;
          text: string;
          at: number;
          /**
           * True when the user sent this message while a turn was already
           * in-flight (isWorking was true before TurnStart). False for
           * idle sends. Stored on the PendingMessage entry and used by
           * PendingMessagesPanel to gate visibility — idle-send messages
           * must never flash in the amber queued zone.
           * See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
           */
          enqueuedWhileBusy: boolean;
      }
    /** Backend acknowledged the message — remove from pending. */
    | { type: "PendingMessageAccepted"; id: string }
    /** RPC failed — remove the entry so user doesn't see a ghost row. */
    | { type: "PendingMessageRejected"; id: string }
    /**
     * Pending entry ageing watchdog fired (caller-scheduled setTimeout).
     * Removes the entry by id; idempotent no-op if already accepted/rejected.
     * Issue #728 gap 2 — prevents zombie pending rows when the backend
     * never acknowledges a queued message (e.g. RPC hang post-send).
     */
    | { type: "PendingMessageExpired"; id: string }

    /**
     * Provider is rate-limited (429) and waiting to retry. Updates
     * `lastEventMs` (keeps the stuck-stream watchdog quiet) and sets
     * `waitingReason` on `Streaming` so the working row can show
     * "Rate limited — retrying…" instead of the thinking phrase.
     * Cleared automatically on the next real activity (bumpEvent).
     */
    | { type: "ProviderWaiting"; reason: "rate_limited"; retryAfterMs: number | null; at: number }

    // ── Failure recovery ──────────────────────────────────────────
    /**
     * Backend classified a failure for this pane (the `agentfailure` wave
     * event). Unconditionally force-ends a working turn (Submitting /
     * Streaming / Interrupting → Done.errored) — this is what closes the
     * "stuck Waiting after a rate-limit interruption" bug: an authoritative
     * failure classification no longer depends on the CLI process actually
     * exiting (persistent-mode agents never do between turns) to clear the
     * turn phase. See SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
     */
    | { type: "FailureObserved"; failure: AgentFailure; at: number }
    /** User dismissed the failure row (Dismiss / ×). Episode over. */
    | { type: "FailureCleared" }

    // ── Composer details / Log panel ─────────────────────────────
    /** Toggle the log panel open/closed. */
    | { type: "DetailsToggle" }
    /** Explicitly open the log panel. Idempotent if already open. */
    | { type: "DetailsExpand" }
    /** Explicitly close the log panel. Idempotent if already closed. */
    | { type: "DetailsCollapse" }

    // ── Compaction (SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md) ──
    /**
     * The `PreCompact` hook fired — compaction has begun. Sourced from
     * the `compaction_started` WPS event (`agentmux-bashwrap precompact`
     * → `useCompactionStream.ts`). Sets `compacting` so the view can
     * show a live "Compacting…" status chip + elapsed counter (Tier
     * 1/2). Also bumps `lastEventMs` like `ProviderWaiting` does, so
     * the stuck-stream watchdog doesn't misfire during a long
     * compaction that produces no other stream activity.
     */
    | { type: "CompactionStarted"; trigger: "manual" | "auto"; at: number }
    /**
     * The real `compact_boundary` frame arrived — compaction finished.
     * Sourced from the backend's `AgentEvent::CompactionBoundary`
     * (translated 1:1 from Claude Code's stream-json `system` frame —
     * exact data, not inferred). Clears `compacting`, records
     * `lastCompactionBoundaryAt` (dedup guard for the `TokensIn`
     * heuristic), and reconciles `lastContextTokens` to `postTokens` so
     * the context-fill bar reflects the real post-compaction size
     * immediately rather than waiting for the next `TokensIn`.
     */
    | {
          type: "CompactionBoundary";
          trigger: "manual" | "auto";
          preTokens: number;
          postTokens: number;
          durationMs: number;
          at: number;
          /**
           * The `compact_boundary` frame's own `timestamp` field, raw
           * (not parsed/reformatted) — absent/`null` if the frame didn't
           * have one (or the caller doesn't have a raw frame at all, e.g.
           * a test constructing this command directly). Threaded through
           * ONLY so the emitted `context-compacted` event's node id can
           * match `parseHistoryLines.ts`'s id for the same persisted line
           * — never used for `state.at`/watchdog purposes, which stay on
           * `Date.now()` (Codex P2, PR #2378 round 7). Optional: every
           * existing call site/test that doesn't have a frame to key on
           * still type-checks unchanged; `pushContextCompactedNodes`
           * falls back to a live timestamp when absent.
           */
          frameTimestamp?: string | null;
      }

    // ── Attached task axis (SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md) ──
    /**
     * Fires on the 0→1 transition of "≥1 PinnedActivity is `running` for
     * this block." No-op (idempotent) if an episode is already in
     * progress — must NOT reset `since` when a second task starts running
     * alongside an already-running one.
     */
    | { type: "AttachedTaskObserved"; at: number }
    /**
     * Fires on the 1→0 transition — the last currently-running
     * `PinnedActivity` for this block just ended. No-op if already null.
     */
    | { type: "AttachedTaskCleared" }
    ;

export type AgentPaneEvent =
    | { type: "init-ready" }
    | { type: "init-failed"; reason: string }
    | { type: "stream-subscribed"; at: number }
    | { type: "stream-unsubscribed"; at: number }
    /**
     * PR F: the stream tore down while a turn was in flight
     * (Submitting / Streaming / Interrupting). Surfaced alongside the
     * generic `stream-unsubscribed` so the view can drive a dedicated
     * "Disconnected — reconnecting…" banner without re-deriving from
     * `turnPhase.kind`. Emitted ONLY on the working-set → Disconnected
     * transition; idle-state unsubscribes do not emit it.
     */
    | {
          type: "stream-disconnected";
          at: number;
          lastKind: KindBeforeDisconnect;
          reason: DisconnectReason;
      }
    | { type: "stream-flush-observed"; addedCount: number }
    | {
          /**
           * Stream has been silent for `idleSinceMs` while subscribed —
           * exceeds `thresholdMs`. Surfaced for diagnostics / UI badge.
           */
          type: "stream-stuck";
          idleSinceMs: number;
          thresholdMs: number;
      }
    | {
          /**
           * The watchdog force-recovered a hung `Streaming` turn to `Idle`
           * after `idleSinceMs` ≥ `LIVENESS_RECOVERY_MS` with no tool active
           * and no rate-limit wait — i.e. a turn whose terminal `session_end`
           * was never observed (the persistent-mode stuck-"Working" class).
           * Surfaced for telemetry; the phase change is the real effect.
           * See SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md.
           */
          type: "working-recovered";
          idleSinceMs: number;
          thresholdMs: number;
      }
    | { type: "turn-started"; at: number }
    /**
     * `ReconcileTurnActive` promoted the phase to `Streaming` because the
     * backend reported a turn already in flight — either the mount-default
     * `Idle` (the original case) or a settled `Done.completed` episode (the
     * focus-triggered reconcile's missed-live-turn-start case). Surfaced
     * for diagnostics — distinguishes this from a normal user-initiated
     * `turn-started`. Not "-at-mount" anymore since it can now fire well
     * after mount.
     */
    | { type: "turn-active-reconciled" }
    /**
     * `ReconcileTurnActive` demoted a stuck `Streaming` phase to `Idle`
     * because the backend's authoritative `turn_active` reported the turn
     * has ended (the `result` event fired) but the frontend never observed
     * the terminal `session_end` — the class behind the Agent1 stuck-"Working"
     * and Agent2 stuck-"Queued" incidents. Completes #2005's reconciliation
     * symmetry (which only promoted Idle→Streaming). Surfaced for telemetry;
     * the phase change is the real effect. See
     * docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md.
     */
    | { type: "turn-inactive-reconciled"; at: number }
    /**
     * `ReconcileContextFromHistory` seeded `lastContextTokens` from a
     * historical `session_end` at mount. Surfaced for diagnostics only.
     */
    | { type: "context-reconciled-at-mount"; tokens: number }
    | {
          /**
           * A turn finished and the phase transitioned to `Done`. Since
           * the sound-notifications subsystem (SPEC_SOUND_NOTIFICATIONS_
           * 2026_06_05.md §3.2), the event carries the outcome directly
           * so downstream consumers (notification sounds, telemetry,
           * future UI affordances) don't have to snapshot the slot to
           * read it back from `state.turnPhase.outcome`. The reducer
           * already computes `outcome` two lines earlier — this is a
           * pure forwarding of an existing value.
           */
          type: "turn-ended";
          outcome: TurnOutcome;
          statsMerged: boolean;
          stoppingCleared: boolean;
      }
    | { type: "turn-reset" }
    | { type: "turn-start-failed" }
    | {
          /**
           * Invariant fire: TurnStart while streaming inactive OR
           * initPhase.kind === "InitPending" — dropped.
           */
          type: "turn-start-suppressed";
          reason: string;
      }
    | { type: "tool-started"; name: string }
    | { type: "tool-ended" }
    | { type: "tokens-updated"; input: number | null; output: number | null }
    /**
     * `source: "real"` — sourced from an authoritative `CompactionBoundary`
     * event (`trigger`/`durationMs` present). `source: "heuristic"` —
     * inferred from a ≥50%-drop in `TokensIn` (the pre-existing
     * cross-provider fallback; `trigger`/`durationMs` absent). See
     * `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §4.3.
     */
    | {
          type: "context-compacted";
          tokensBefore: number;
          tokensAfter: number;
          source: "real" | "heuristic";
          trigger?: "manual" | "auto";
          durationMs?: number;
          /** `source: "real"` only — see `CompactionBoundary` command's doc comment. */
          frameTimestamp?: string | null;
      }
    /** `CompactionStarted` landed — compaction is in progress. */
    | { type: "compaction-started"; trigger: "manual" | "auto" }
    | { type: "provider-waiting"; reason: "rate_limited" }
    /**
     * A failure was observed and recorded on `state.failure`. `turnWasEnded`
     * is true iff the pane's turn was actually working (Submitting /
     * Streaming / Interrupting) at the time — surfaced for diagnostics
     * (e.g. distinguishing a genuine mid-turn failure from a stray late
     * event arriving after the pane was already idle).
     */
    | { type: "failure-observed"; code: AgentFailure["code"]; turnWasEnded: boolean }
    /** `state.failure` was cleared (explicit dismiss, or implicitly by TurnStart). */
    | { type: "failure-cleared" }
    | { type: "stop-requested"; at: number }
    | { type: "stop-failed" }
    /**
     * Reducer signal: a turn has just entered Interrupting. The
     * dispatch layer (view / saga) is expected to start a setTimeout
     * for `INTERRUPT_TIMEOUT_MS` and fire `InterruptTimeoutElapsed`
     * when it expires — gated by a shared cancel flag (analogous to
     * `Arc<AtomicBool>`) so a later `RequestStop` doesn't double-fire.
     * `deadlineMs` is `at + INTERRUPT_TIMEOUT_MS` so the caller can
     * compute the remaining slice deterministically. Spec §8.
     */
    | {
          type: "schedule-interrupt-timeout";
          deadlineMs: number;
      }
    /**
     * Reducer signal: the bounded `Interrupting → Done.{interrupted}`
     * force-transition fired because no graceful ack arrived within
     * `INTERRUPT_TIMEOUT_MS`. The view can surface this as
     * "interrupt timed out — assuming dead" if needed. Spec §8.
     */
    | { type: "interrupt-timed-out"; at: number }
    /**
     * Reducer signal: a turn has just entered Submitting. The dispatch
     * layer is expected to start a setTimeout for `SUBMIT_TIMEOUT_MS`
     * and fire `SubmitTimeoutElapsed` when it expires — gated by a
     * shared cancel flag so a graceful promotion to Streaming (via
     * `StreamFlushObserved` or `bumpEvent` from tool / token activity)
     * cancels the timer before it fires. `deadlineMs` is
     * `at + SUBMIT_TIMEOUT_MS` so the caller can compute the remaining
     * slice deterministically. PR D — spec §8 / issue #728 gap 2.
     */
    | {
          type: "schedule-submit-timeout";
          deadlineMs: number;
      }
    /**
     * Reducer signal: the bounded `Submitting → Done.{errored}`
     * force-transition fired because no stream activity arrived within
     * `SUBMIT_TIMEOUT_MS`. Carries the firing wall-clock for audit.
     * PR D — spec §8 / issue #728 gap 2.
     */
    | { type: "submit-timed-out"; at: number }
    | { type: "pending-queued"; id: string }
    | { type: "pending-accepted"; id: string; wasPresent: boolean }
    | { type: "pending-rejected"; id: string; wasPresent: boolean }
    | {
          /** Pending entry expired and was removed; queuedAt for log/audit. */
          type: "pending-expired";
          id: string;
          queuedAt: number;
          ageMs: number;
          wasPresent: boolean;
      }
    /**
     * Emitted when the pane enters the waiting-for-input state: turn
     * completed with a question, composer empty. The sound service
     * starts the looping ambient tone on this event.
     * Spec: SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md §6.3.
     */
    | { type: "waiting-for-input" }
    /**
     * Emitted when the waiting state ends — user submitted or started
     * typing, pane closed, or the 5-minute safety cutoff fired.
     */
    | { type: "waiting-ended"; reason: "submitted" | "typing" | "closed" }
    /** `AttachedTaskObserved` set `state.attachedTask` (0→1 edge). */
    | { type: "attached-task-observed"; at: number }
    /** `AttachedTaskCleared` cleared `state.attachedTask` (1→0 edge). */
    | { type: "attached-task-cleared" };

export interface ReducerResult {
    state: AgentPaneState;
    events: AgentPaneEvent[];
}

/**
 * Stuck-stream threshold. A subscribed stream that hasn't received any
 * observable event (subscribe, flush, tool, tokens) for this many ms is
 * flagged. 45s covers the typical Claude/Codex idle window between
 * thinking and tool calls during long reasoning runs without false
 * positives. Issue #728 gap 3.
 */
export const STUCK_THRESHOLD_MS = 45_000;

/**
 * Liveness-recovery threshold. A `Streaming` turn that has produced no
 * observable activity for this many ms — with no tool running and not
 * rate-limited — is treated as hung: its terminal `session_end` was never
 * observed and no other signal will arrive to clear it. For persistent-mode
 * agents the process never exits between turns, so `ControllerStatus: done`
 * never fires and the process-exit grace timer can't recover the phase; the
 * watchdog itself must transition out of "Working" (→ `Idle`). 3 min is well
 * past `STUCK_THRESHOLD_MS` (45s) so a merely-quiet reasoning run is never
 * mistaken for a hang. See SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md.
 */
export const LIVENESS_RECOVERY_MS = 180_000;

/**
 * Bounded `Interrupting → Done.{interrupted}` window. After `RequestStop`
 * fires SIGINT, if neither `TurnEnd` nor `StreamUnsubscribe` lands within
 * this many ms the reducer force-transitions to Done.interrupted on
 * receipt of `InterruptTimeoutElapsed`. 5 s is generous compared to the
 * typical sub-second SIGINT-ack on a healthy agent but tight enough that
 * a dead agent doesn't keep the working animation pinned forever.
 *
 * Spec: SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §8 — the spec doc
 * suggests 3_000 but the implementation lands at 5_000 to leave more
 * headroom for slow Windows PTY teardowns and remote agents on flaky
 * networks. Tune in a follow-up if telemetry shows the watchdog firing
 * on live agents.
 */
export const INTERRUPT_TIMEOUT_MS = 5_000;

/**
 * Bounded `Submitting → Done.{errored}` window. After `TurnStart` puts a
 * turn into Submitting, if no stream activity (flush / bumpEvent from
 * tool / token) lands within this many ms the reducer force-transitions
 * to Done.errored on receipt of `SubmitTimeoutElapsed`. 30 s is the
 * generous outer bound for a backend ack — network handshakes, agent
 * spawn, and the first model token round-trip on a fresh session. Any
 * longer and the working animation has been pinned past the user's
 * patience window; PR E's streaming watchdog covers the longer "agent
 * is thinking" idle case once we're in Streaming.
 *
 * PR D — spec §8 / issue #728 gap 2. The dispatcher arms this via the
 * `schedule-submit-timeout` event emitted when `TurnStart` promotes the
 * phase into Submitting; the standard cancel-flag pattern (the same
 * `Arc<AtomicBool>` analogue used for the interrupt timeout) gates a
 * stale fire against a graceful promotion to Streaming.
 */
export const SUBMIT_TIMEOUT_MS = 30_000;

/**
 * Suppression window for the `TokensIn` ≥50%-drop compaction heuristic
 * after a REAL `CompactionBoundary` event landed. Claude Code's
 * `compact_boundary` frame precedes the next turn's `message_start` in
 * the same stream, so `lastContextTokens` is normally already
 * reconciled to `postTokens` before the heuristic's next check runs —
 * this window is a defensive backstop against any ordering surprise or
 * a `postTokens` that doesn't closely match the next observed
 * `TokensIn.input`. 2 minutes comfortably covers one turn's round trip
 * without risking suppressing a LATER, genuinely new compaction. See
 * `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §4.3.
 */
export const COMPACTION_HEURISTIC_SUPPRESS_MS = 120_000;
