// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the agent-pane-state reducer (slice #4 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md): the per-pane atoms with cohesive
 * cross-atom invariants (streaming, sessionStats, currentTool, turnTokens, pendingMessages) plus
 * the init phase, turn phase and stream-watchdog fields (#728).
 *
 * The legacy `turnActive`/`stopping`/`streaming.active` booleans are gone: views read
 * `isWorking(state)` / `state.turnPhase.kind` / `isDisconnected(state)`, and the reducer's "is the
 * stream subscribed?" gate is `state.lastEventMs !== null`.
 *
 * The agent document (message list) is its own slice (`agent-document/`): no god-reducer
 * (conventions doc §11).
 *
 * Why each field, command and event is the way it is: docs/architecture/AGENT_PANE_STATE_NOTES.md,
 * one entry per member, headed by its name.
 */

import type { PendingMessage } from "../../agent-model/state";
import type { ContextReading, ContextSource, ContextWindowMap, ReportedContextWindows } from "./context-reading";
import type { FailureSource } from "./failure-precedence";
import type { TurnLedger } from "./turn-ledger";
import type { ToolActivity } from "@/app/store/agent-pane-state/tool-labels";
import type { PlanState } from "@/app/store/agent-pane-state/plan";

/** What the model is doing within a pass, by what its stream shows. */
export type ModelActivityPhase = "requesting" | "responding" | "thinking" | "writing" | "composing";

/**
 * What the agent is doing right now, for the working row's live status
 * (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.3). Only facts the
 * stream reports; the presenter (view/agent/status/present-status.ts)
 * decides what to say and when.
 */
export interface ActivityState {
    /** The model's phase, and since when; null outside a pass. */
    phase: ModelActivityPhase | null;
    phaseSince: number;
    /** Tool calls in flight, oldest first. A subagent's own calls carry
     *  `parentId` (its Agent call's id); `progress` is what a known test
     *  runner's output says so far. */
    tools: Array<{
        id: string | null;
        activity: ToolActivity;
        /** The tool's name and, for a shell call, its command: what sorts it into a kind of long-running work. */
        name?: string;
        command?: string;
        startedAt: number;
        parentId?: string;
        progress?: string;
        /** When it last wrote output (live chunks; throttled). Absent: none yet. */
        outputAt?: number;
    }>;
    /** What the model's current thinking block is about, from its own text. */
    thinkingHeadline?: string;
    /** This pane's recent waits for the model (request sent → its answer
     *  began), newest last, at most WAIT_SAMPLES: what "slow" means here. */
    waits?: number[];
    /** The agent's plan from its latest todo list, and when it was set. Kept
     *  across passes; the presenter reads it only when set this turn. */
    plan: PlanState | null;
    planAt: number;
}

export const IDLE_ACTIVITY: ActivityState = { phase: null, phaseSince: 0, tools: [], plan: null, planAt: 0 };
import type {
    SessionStats,
    StreamingState,
    TurnTokens,
} from "../../agent-model/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";

/**
 * Init lifecycle, the "history still loading" gap (#728 gap 1): `InitPending` → `InitReady` or
 * `InitFailed`. One-way: once terminal, `InitStart` is a no-op, and a re-mount makes a new pane
 * slot back at `InitPending`.
 */
export type InitPhase =
    | { kind: "InitPending" }
    /**
     * `historyDamaged`: srv found the agent's saved history in the shared store damaged and
     * answered from the block's own copy, which holds only what this block wrote.
     */
    | { kind: "InitReady"; at: number; historyDamaged?: true }
    | { kind: "InitFailed"; at: number; reason: string };

// TurnPhase: the single source of truth for the turn lifecycle
// (docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5). Consumers project via `isWorking` /
// `isDisconnected` / `state.turnPhase.kind`.

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
 * Why the stream dropped mid-turn. `stream-unsubscribed`: a clean `StreamUnsubscribe` (unmount, PTY
 * exit). `transport-error`: reserved, no caller wires it yet.
 */
export type DisconnectReason = "stream-unsubscribed" | "transport-error";

/**
 * Live "compaction in progress" state: set when the `PreCompact` hook's `compaction_started` lands,
 * cleared by the matching `compact_boundary` (a real `CompactionBoundary`). `startedAt` drives the
 * live elapsed counter.
 */
export interface CompactionState {
    trigger: "manual" | "auto";
    startedAt: number;
}

/**
 * Live "≥1 agent-declared long-running task is attached" state, a sibling axis independent of
 * `turnPhase` (it must outlive the turn lifecycle). Sourced from the dock's aggregated
 * `PinnedActivity` list, not OS process counts. `since` is the start of the current unbroken "≥1
 * running" episode; a second task starting doesn't reset it.
 */
export interface AttachedTaskState {
    since: number;
}

/**
 * Live "controller is recovering from a stale `--resume` session id" state, or `null`: set when the
 * stale-resume retry begins (`{status:"retrying"}`), cleared once the outcome is known
 * (`{status:"resolved"}`).
 */
export interface ResumeRetryState {
    startedAt: number;
}

/**
 * A classified backend failure (the `agentfailure` event's `AgentFailure`) for this pane: the
 * single source of truth for "is there an active failure". It does NOT carry the auto-retry
 * countdown/budget or the expanded/retrying view flags (hook-local in `useAgentFailure.ts`).
 */
export interface PaneFailure {
    /** The classified failure, verbatim from the backend event. */
    data: AgentFailure;
    /** Wall-clock ms the failure landed. */
    at: number;
    /**
     * Whether a turn was attempted before this failure: `true` for every backend-classified
     * failure, `false` only for the synthetic pre-launch auth failure. Load-bearing: it selects
     * `relogin`'s `retryAfterLogin`, and `true` for a never-started agent would re-send its last
     * OLD message after login. Defaults to `true` when absent.
     */
    turnAttempted?: boolean;
    /**
     * Who raised it (failure-precedence.ts): a failure of a lower-ranked source never replaces
     * this one. Absent means the backend's event, as for a record that predates sources.
     */
    source?: FailureSource;
}

/** Why the failure row went (for the event, and so each clear says what it is). */
export type FailureClearReason = "dismissed" | "retried" | "episode-ended" | "recovered" | "retracted" | "offline";

/**
 * True when the pane's failure is an auth failure: the agent isn't signed in
 * to its provider, or its sign-in stopped working. The one spelling of the
 * check the send path, the relogin flow and the controller-status handling
 * each wrote out by hand (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §5.2).
 */
export function isAuthFailure(failure: Pick<PaneFailure, "data"> | null | undefined): boolean {
    return failure?.data.code === "auth";
}

/**
 * The turn lifecycle: the only encoding of "is the agent working", "is a stop in flight" and
 * "did the stream drop".
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
    | { kind: "Submitting"; submittedAt: number; pendingContent: string; hidden?: boolean }
    | {
          kind: "Streaming";
          bufferSize: number;
          toolsActive: number;
          lastEventMs: number;
          /** Set when the provider is rate-limited and retrying. Cleared on next real activity. */
          waitingReason?: "rate_limited";
          /** Milliseconds until the next retry, from the provider's Retry-After header. */
          retryAfterMs?: number | null;
          /** When that retry is due (epoch ms): `at` of the ProviderWaiting
           *  plus `retryAfterMs`. A countdown reads this; `retryAfterMs` alone
           *  is a fixed duration and never counts down. */
          retryAtMs?: number | null;
          /**
           * The model ended its turn (`stop_reason: end_turn`) but the turn is still open;
           * absent/false means it is still working. Set by `ModelEndedTurn`, cleared by
           * `ModelMessageStarted`; a new turn starts without it. The busy predicate's
           * background-work carve-out applies only when this is true.
           */
          modelEndedTurn?: boolean;
      }
    | {
          kind: "Interrupting";
          reason: InterruptReason;
          sigintSentAt: number;
      }
    | { kind: "Done"; outcome: TurnOutcome; finishedAt: number }
    | {
          /**
           * Stream dropped while a turn was in flight. `lastKind`: the working-kind we lost;
           * `lastConnectedAt`: ms the disconnect command was processed; `reason`: see {@link
           * DisconnectReason}.
           */
          kind: "Disconnected";
          lastKind: KindBeforeDisconnect;
          lastConnectedAt: number;
          reason: DisconnectReason;
      };

/** See `AgentPaneState.turnCarry`. */
export interface TurnCarry {
    turnId: number;
    passes: number;
    outputTokens: number;
    costUsd: number;
    steps: number;
}

/**
 * The reducer's state. Each field maps 1:1 to a Solid signal the agent pane projects from; the
 * reducer enforces invariants ACROSS fields (e.g. no turn enters Submitting/Streaming while the
 * stream is unsubscribed: the `TurnStart` arm). `state.turnPhase.kind` is the sole working-state
 * encoding; "is the stream subscribed?" is `state.lastEventMs !== null` (cleared atomically with
 * the stream on `StreamSubscribe`/`StreamUnsubscribe`).
 */
export interface AgentPaneState {
    streaming: StreamingState;
    sessionStats: SessionStats | null;
    /**
     * Cumulative cost/tokens/duration across every completed turn in this pane's lifetime: unlike
     * `sessionStats` (replaced on each TurnEnd) it sums into the previous value. Feeds the
     * composer-strip "totals" display; cleared only on TurnReset
     * (SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md).
     */
    sessionTotals: SessionStats | null;
    currentTool: string | null;
    /** First significant argument of the active tool (file path, command, etc.).
     *  Cleared alongside currentTool on ToolEnd / TurnEnd / TurnReset. */
    currentToolArg: string | null;
    /**
     * The current turn's live usage: set by `TokensIn`; cleared by that turn's `TurnEnd`, by
     * `StreamSessionStarted` (the process died before a `result`) and by hard ends (disconnect,
     * failure, timeouts, reset). Nothing else clears it on a normal turn end: srv sends the
     * turn-ended push before the `result` line, so `session_end` (and sometimes earlier calls)
     * still follow, and a queued turn can start in that gap.
     */
    turnTokens: TurnTokens | null;
    /**
     * The turn as the user sees it over however many CLI passes: srv's `agentturn` event, verbatim
     * (turn-ledger.ts); null until srv has published one. `turnPhase` and `turnTokens` stay per
     * pass.
     */
    turnLedger: TurnLedger | null;
    /** What the agent is doing right now (see `ActivityState`). */
    activity: ActivityState;
    /**
     * Each pass's own figures (its `TurnEnd` stats), summed over the turn srv reported it in; the
     * Worked line reports from this for providers whose passes srv doesn't count (ACP, App Server,
     * one-shot subprocess).
     */
    turnCarry: TurnCarry | null;
    /**
     * True for a turn started to send a manual "/compact": set on `TurnStart` when `command.content
     * === "/compact"`, consumed by `CompactionBoundary`. A manual boundary's `trigger` alone is NOT
     * a safe signal that this pane's turn is the synthetic one (the events have no ordering
     * guarantee against the NDJSON stream) (#2659).
     */
    pendingCompactTurn: boolean;
    pending: PendingMessage[];
    /**
     * Init phase: `InitPending` until history fetch resolves or fails; the failure reason lives in
     * `InitFailed`'s payload (no separate `initError`).
     */
    initPhase: InitPhase;
    /**
     * Ms of the last observable stream activity (subscribe, flush, tool, tokens); drives the
     * stuck-stream watchdog. Also the "is the stream subscribed?" gate: `lastEventMs !== null` ⇔ a
     * `StreamSubscribe` that no later `StreamUnsubscribe` has cleared.
     */
    lastEventMs: number | null;
    /**
     * Single-source-of-truth turn phase; consumers use `isWorking(state)` / `isDisconnected(state)`
     * / `state.turnPhase.kind`.
     */
    turnPhase: TurnPhase;

    /**
     * Whether the composer details panel (the Shell drawer with the interactive terminal) is open.
     * Default `false`, per session, reset on pane unmount, not persisted. It must NOT auto-collapse
     * on `TurnStart`: the live shell (AgentShellSubblock) has to survive sending more messages.
     */
    detailsOpen: boolean;
    /**
     * The context meter's reading: tokens, the model's window and where each came from
     * (context-reading.ts). Set by `TokensIn` and, only while `contextSeedable`, by
     * `ReconcileContextFromHistory`. Unlike `turnTokens` it is NOT cleared at TurnEnd; it is
     * cleared on TurnReset, ContextInvalidated and CompactionBoundary. Displays read it through
     * `plausibleReading`, never directly.
     */
    context: ContextReading | null;
    /**
     * True until something more current than the restored transcript has
     * spoken: a live reading, an invalidation, a compaction or a reset.
     * `ReconcileContextFromHistory` seeds only while it holds — `context`
     * being null can't tell "nothing yet" from "invalidated while history was
     * still loading", and seeding the latter restores a dead conversation.
     */
    contextSeedable: boolean;
    /**
     * Context windows Claude Code reported in `result.modelUsage`, per model
     * id. They outrank the model-name table for any reading on that model, and
     * outlive TurnReset: a model's window is a fact about the model.
     */
    reportedContextWindows: ReportedContextWindows;
    /**
     * Context windows proven per model by an accepted prompt larger than the
     * window we had (context-reading.ts `learnedWindowsAfter`). Outlive
     * TurnReset for the same reason as `reportedContextWindows`.
     */
    learnedContextWindows: ContextWindowMap;
    /** Resolved model id of the main agent's latest reply (`message.model`).
     *  The Runtime menu shows it; the context reading is measured on it. */
    lastContextModel: string | null;
    /**
     * Active classified failure for this pane, or `null`. Set by `FailureObserved` (which also
     * force-ends a working turn), cleared by `FailureCleared` or implicitly by the next `TurnStart`
     * (a fresh turn ends the failure episode). See {@link PaneFailure}.
     */
    failure: PaneFailure | null;
    /** Live "compaction in progress" state, or null. See {@link CompactionState}. */
    compacting: CompactionState | null;
    /**
     * Live "attached long-running task" state, or null ({@link AttachedTaskState}). Deliberately
     * NOT cleared by any `TurnPhase` transition: a task attached in one turn can outlive it, so
     * only `AttachedTaskObserved`/`AttachedTaskCleared` touch it.
     */
    attachedTask: AttachedTaskState | null;
    /**
     * Live "reconnecting after a stale `--resume` session id" state, or
     * `null` — see {@link ResumeRetryState}. Owned exclusively by
     * `ResumeRetryStarted`/`ResumeRetryResolved`, dispatched from
     * `useResumeRetryStream.ts`.
     */
    reconnecting: ResumeRetryState | null;
    /**
     * Earliest `started_at_ms` among this block's `Running` `db_background_tasks` rows, or `null`:
     * a separate floor signal from `attachedTask` (the durable registry can know a task the live
     * transcript doesn't). Owned EXCLUSIVELY by `RegistryAttachedTaskObserved`/`Cleared`
     * (dispatched only from `useBackgroundTaskRegistry.ts`); the transcript-derived `attachedTask`
     * effect (`agent-view.tsx`) only READS it and combines both, earliest start winning (#2685).
     */
    registryAttachedTaskSince: number | null;
    /**
     * Ms of the most recent REAL `CompactionBoundary` event. The `TokensIn` ≥50%-drop heuristic
     * checks it before firing its own synthetic `context-compacted`, so a session that just got a
     * real boundary gets no duplicate. Providers without a structured signal never set it.
     */
    lastCompactionBoundaryAt: number | null;
    /**
     * A `compaction_started` ping rejected because `turnPhase` wasn't (yet) working, which a later
     * authoritative signal can still promote for THIS turn (`ReconcileTurnActive(active: true)` or
     * `StreamFlushObserved`'s promotion arm). Cleared on that promotion, when a signal proves the
     * turn isn't running, by a fresh unrelated `TurnStart`, or by any terminal transition that
     * clears `compacting`.
     */
    pendingCompactionPing: { trigger: "manual" | "auto"; startedAt: number } | null;
}

export const initialState = (agentId: string): AgentPaneState => ({
    streaming: { agentId, bufferSize: 0, lastEventTime: 0 },
    sessionStats: null,
    sessionTotals: null,
    currentTool: null,
    currentToolArg: null,
    turnTokens: null,
    turnLedger: null,
    turnCarry: null,
    activity: IDLE_ACTIVITY,
    pendingCompactTurn: false,
    context: null,
    contextSeedable: true,
    reportedContextWindows: {},
    learnedContextWindows: {},
    lastContextModel: null,
    pending: [],
    initPhase: { kind: "InitPending" },
    lastEventMs: null,
    turnPhase: { kind: "Idle" },
    detailsOpen: false,
    failure: null,
    compacting: null,
    attachedTask: null,
    reconnecting: null,
    registryAttachedTaskSince: null,
    lastCompactionBoundaryAt: null,
    pendingCompactionPing: null,
});

/** Selector — `true` iff the pane has finished its initial history load. */
export function isInitReady(state: AgentPaneState): boolean {
    return state.initPhase.kind === "InitReady";
}

/**
 * True while the agent is processing a user request: `state.turnPhase.kind ∈ {Submitting,
 * Streaming, Interrupting}` (Idle/Done/Disconnected are not working). The only "working" predicate
 * (SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §7).
 */
export function isWorking(state: AgentPaneState): boolean {
    return workingFromPhase(state.turnPhase);
}

/**
 * Phase-only variant of {@link isWorking}, for call sites that have the phase but not the whole
 * state (the view projects `turnPhase` through its own signal). Same predicate.
 */
export function workingFromPhase(phase: TurnPhase): boolean {
    const k = phase.kind;
    return k === "Submitting" || k === "Streaming" || k === "Interrupting";
}

/** A stop is in flight (`Interrupting`). Still counts as working. */
export function isStopping(phase: TurnPhase): boolean {
    return phase.kind === "Interrupting";
}

/**
 * True iff the pane is in the `Disconnected` phase; drives the {@link AgentDisconnectedBanner}. A
 * pure projection of `turnPhase.kind`, the same source of truth as `isWorking`
 * (docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §6.4).
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
    | { type: "InitReady"; at: number; historyDamaged?: boolean }
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
    | {
          type: "StreamFlushObserved";
          addedCount: number;
          at: number;
          /**
           * Did this flush carry evidence the TURN is alive: document nodes, node updates, tool
           * chunks or a new shell node? `false` means shell OUTPUT only, a live BACKGROUND process
           * writing to its dock row, which says nothing about the model, so it must not refresh the
           * turn idle clock or resurrect a finished turn. Omitted counts as `true`; only the flush
           * queue knows the difference and sets this
           * (docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md).
           */
          turnRelevant?: boolean;
      }
    /**
     * Periodic tick from useAgentStream's watchdog interval. Emits a `stream-stuck` diagnostic past
     * `STUCK_THRESHOLD_MS` and, past the longer `LIVENESS_RECOVERY_MS`, force-recovers a hung
     * `Streaming` turn (no tool active, not rate-limited) to `Idle`, clearing a stuck "Working"
     * that never got its `session_end` (SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md).
     */
    | { type: "StreamWatchdogTick"; nowMs: number }

    /**
     * Reconciliation from the backend's `turn_active` (health monitor on the NDJSON stream),
     * fetched at mount and on every `controllerstatus` MPS event. `active: true` promotes only an
     * `Idle` phase to `Streaming`; a phase a real stream or user event already produced wins.
     * `active: false` demotes only a stuck `Streaming` to `Idle` (the backend flips it only on the
     * CLI's `result`, so it is authoritative); `Submitting` and `Interrupting` have their own
     * timeouts.
     */
    | { type: "ReconcileTurnActive"; at: number; active: boolean }
    /**
     * Mount-time seed of `context` from the history's last main-agent API call. Seeds only while
     * `contextSeedable`: a historical snapshot must never clobber real-time data. An implausible
     * seed is refused (`context-reading-rejected`); `tokens` is absent when the window holds no
     * main-agent call; `reportedWindows` merge UNDER any reported live.
     */
    | {
          type: "ReconcileContextFromHistory";
          tokens?: number | null;
          model?: string | null;
          /** When that call was received (unix ms), if the line was stamped. */
          at?: number | null;
          reportedWindows?: ReportedContextWindows | null;
      }
    /**
     * A `result` frame reported context windows (`modelUsage[*].contextWindow`).
     * Recorded per model; the current reading's window is re-resolved.
     */
    | { type: "ContextWindowsReported"; windows: ReportedContextWindows }
    /**
     * An agent reported how full its context is (view/agent/context-reader.ts):
     * the meter's reading, on `model` (else the last one seen), with the
     * window it was measured against when it said. Unlike `TokensIn` it starts
     * no call, so the turn's counters are untouched.
     */
    | { type: "ContextReported"; tokens: number; model?: string; window?: number }
    /**
     * The conversation the reading measured is gone (a `fresh` session, or the transcript
     * archived/restored). Clears `context` and ends `contextSeedable`, so the next reading isn't
     * read as a compaction and a late history seed can't bring the old conversation back.
     */
    | { type: "ContextInvalidated"; reason: "fresh_session" | "transcript_replaced" }
    /**
     * The pane's model setting changed (`/model`, the model menu). The conversation, and so the
     * reading's tokens, carry over; the window becomes unknown until the new model's first reply
     * reports it (`model` is often a family alias such as `sonnet`, which doesn't say its window).
     */
    | { type: "ContextModelSwitched"; model: string }
    /**
     * User pressed send: the turn becomes active and the previous turn's stale `sessionStats` are
     * cleared. `content` is the submitted text, threaded into `TurnPhase.Submitting.pendingContent`
     * (optional so test call sites compile; the reducer defaults to `""`).
     */
    | {
          type: "TurnStart";
          at: number;
          content?: string;
          /**
           * True for a hidden memory-reinjection turn, never set by an ordinary send. Propagated
           * onto `TurnPhase.Submitting.hidden` so every ambient hook keyed off `turnPhase` skips
           * its own LLM call for the turn (an ambient summary of the injected text once became the
           * visible pane title, #3502). `content` for a hidden turn is also never the real message.
           */
          hidden?: boolean;
      }
    /**
     * Stream produced session_end (or the fallback timer fired). Final stats merge with the current
     * turn tokens, which are still there when the backend's turn-ended push already moved the phase
     * to Idle. Clears currentTool and turnTokens; the phase becomes Done (Interrupting →
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
    /** srv published the block's turn ledger (`agentturn`). An older turn's,
     *  or an older state of this one, landing late is ignored. */
    | { type: "TurnObserved"; ledger: TurnLedger }
    /**
     * Revert an OPTIMISTIC `TurnStart` when the turn never began (the send's RPC failed
     * synchronously, or a slash/bang command was handled locally). Phase only: unlike `TurnReset`
     * it must NOT touch `sessionStats` / `sessionTotals` / `context`, which accumulate over the
     * pane's lifetime (#2318).
     */
    | { type: "TurnStartFailed" }

    // ── Tool ───────────────────────────────────────────────────────
    /** `id`: the tool_use id, so its end removes exactly it; `params`: its
     *  input, for the live status's words (tool-labels.ts). */
    | { type: "ToolStart"; name: string; arg?: string; id?: string; params?: Record<string, unknown>; parentId?: string }
    /** A running call's test progress, from its live output (status/test-progress.ts). */
    | { type: "ToolProgress"; id: string; text: string }
    /** A running call wrote output (at most every few seconds per call). */
    | { type: "ToolOutput"; id: string; at: number }
    /** The model's thinking block's gist (status/thinking-headline.ts). */
    | { type: "ThinkingHeadline"; text: string }
    | { type: "ToolEnd"; id?: string }

    // freshInput/cacheCreation/cacheRead: optional breakdown of `input` by
    // cache status (fresh + cacheCreation + cacheRead === input). See
    // TurnTokens'/SessionStats' matching doc comments in agent-model/stream-events.ts —
    // this is the same split, just on the dispatched command.
    | { type: "TokensIn"; input: number; model?: string; freshInput?: number; cacheCreation?: number; cacheRead?: number }
    | { type: "TokensOut"; output: number }
    /** The stream shows a new CLI session starting (`system/init`, not the one
     *  a compaction writes mid-turn; view/agent/session-start.ts): any turn
     *  still holding tokens ended without a `result` (the process died), so
     *  they belong to no turn that is coming. */
    | { type: "StreamSessionStarted" }
    /** Characters the main agent's latest call streamed (text, thinking, tool
     *  input) since the last one: its output estimate until `TokensOut`. */
    | { type: "OutputStreamed"; chars: number; kind?: "text" | "thinking" | "tool_input" }
    /** The main agent's tool results went back: a request is in flight (↑)
     *  until the next call's `TokensIn`. */
    | { type: "RequestStarted" }
    /** The main agent's model sent `stop_reason: end_turn`; see `modelEndedTurn` on the Streaming phase. */
    | { type: "ModelEndedTurn" }
    /** The main agent's model began a new message; it is working again. */
    | { type: "ModelMessageStarted" }

    /** User pressed Esc / clicked Stop. */
    | { type: "RequestStop"; at: number }
    /** Stop RPC failed — bail on the stopping state. */
    | { type: "StopFailed" }
    /**
     * Interrupt timeout watchdog fired: bounded `Interrupting → Done` force transition when the
     * agent never acks the SIGINT. A no-op once the phase has left Interrupting. Spec §8.
     */
    | { type: "InterruptTimeoutElapsed"; at: number }
    /**
     * Submit timeout watchdog fired: bounded `Submitting → Done.errored` force transition when the
     * backend never acks a TurnStart within `SUBMIT_TIMEOUT_MS` (#728 gap 2). A no-op once the
     * phase has left Submitting. Spec §8.
     */
    | { type: "SubmitTimeoutElapsed"; at: number }

    | {
          type: "PendingMessageQueued";
          id: string;
          text: string;
          at: number;
          /** Images sent with the message (ids + names, tray order). */
          attachments?: { id: string; name: string }[];
          /** The pane sends it on AgentMux's behalf (the start-up document): a retry sends it so again. */
          system?: boolean;
          /**
           * True when the user sent this while a turn was in flight (isWorking was true before
           * TurnStart); false for idle sends. Stored on the PendingMessage entry;
           * PendingMessagesPanel uses it so an idle-send message never flashes in the amber queued
           * zone.
           */
          enqueuedWhileBusy: boolean;
      }
    /**
     * A held message is about to be delivered (dispatched right before `deliverToBackend`: the
     * moment sending actually begins). Marks the entry `flushing` so PendingMessagesPanel stops
     * claiming "sends at the agent's next step".
     */
    | { type: "PendingMessageFlushStarted"; id: string }
    /** Backend acknowledged the message — remove from pending. */
    | { type: "PendingMessageAccepted"; id: string }
    /** RPC failed — remove the entry so user doesn't see a ghost row. */
    | { type: "PendingMessageRejected"; id: string }
    /**
     * Pending entry ageing watchdog fired (caller-scheduled setTimeout). Removes the entry by id;
     * idempotent. Prevents zombie pending rows when the backend never acknowledges a queued message
     * (RPC hang post-send; #728 gap 2).
     */
    | { type: "PendingMessageExpired"; id: string }

    /**
     * Provider is rate-limited (429) and waiting to retry. Updates `lastEventMs` (keeps the
     * stuck-stream watchdog quiet) and sets `waitingReason` on `Streaming` so the working row can
     * say "Rate limited — retrying…". Cleared by the next real activity (bumpEvent).
     */
    | { type: "ProviderWaiting"; reason: "rate_limited"; retryAfterMs: number | null; at: number }

    // ── Failure recovery ──────────────────────────────────────────
    /**
     * Backend classified a failure for this pane (the `agentfailure` event). Force-ends a working
     * turn unconditionally (→ Done.errored): an authoritative failure doesn't depend on the CLI
     * process exiting, which persistent-mode agents never do between turns.
     */
    | {
          type: "FailureObserved";
          failure: AgentFailure;
          at: number;
          /**
           * See {@link PaneFailure.turnAttempted}. Omitted by the backend-event
           * path (a classified failure always FOLLOWS a turn that ran), set to
           * `false` only by the pane's own synthetic pre-launch auth failure.
           */
          turnAttempted?: boolean;
          /**
           * Who raises it. Omitted means the backend's event (`live`). Dropped, with no other
           * effect, when the failure showing outranks it (failure-precedence.ts).
           */
          source?: FailureSource;
          /**
           * For a `seed` only: when the pane mounted. A seed is dropped when a turn is running or
           * has finished since, so a stored failure that arrives late never ends or overwrites it.
           */
          since?: number;
      }
    /**
     * The failure row goes: dismissed, its episode over, proven resolved, or retracted. `onlyCode`
     * clears it only when it is that class, decided here in the reducer so no caller reads the
     * failure and then clears a different one that landed in between.
     */
    | { type: "FailureCleared"; reason?: FailureClearReason; onlyCode?: AgentFailure["code"] }

    // ── Composer details / Log panel ─────────────────────────────
    /** Toggle the log panel open/closed. */
    | { type: "DetailsToggle" }
    /** Explicitly open the log panel. Idempotent if already open. */
    | { type: "DetailsExpand" }
    /** Explicitly close the log panel. Idempotent if already closed. */
    | { type: "DetailsCollapse" }

    // ── Compaction (SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md) ──
    /**
     * The `PreCompact` hook fired: compaction has begun (the `compaction_started` MPS event). Sets
     * `compacting` and bumps `lastEventMs` so the stuck-stream watchdog doesn't misfire during a
     * long compaction.
     */
    | { type: "CompactionStarted"; trigger: "manual" | "auto"; at: number }
    /**
     * The real `compact_boundary` frame arrived: compaction finished (exact, not inferred). Clears
     * `compacting`, records `lastCompactionBoundaryAt`, and reconciles `context` to `postTokens`.
     */
    | {
          type: "CompactionBoundary";
          trigger: "manual" | "auto";
          preTokens: number;
          postTokens: number;
          durationMs: number;
          at: number;
          /**
           * The frame's own `timestamp`, raw; absent/`null` when the frame has none or the caller
           * has no raw frame. A time value only (the stale-boundary checks and the node's time,
           * falling back to `at`); never an id: see `boundaryUuid`.
           */
          frameTimestamp?: string | null;
          /**
           * The frame's own `uuid`: keys the emitted `context-compacted`
           * node's id, so it matches `parseHistoryLines.ts`'s id for the
           * same line. Optional for callers without a frame.
           */
          boundaryUuid?: string | null;
      }

    // ── Attached task axis (SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md) ──
    /**
     * Fires on the 0→1 transition of "≥1 PinnedActivity is `running` for this block". Idempotent:
     * it must NOT reset `since` when a second task starts running alongside.
     */
    | { type: "AttachedTaskObserved"; at: number }
    /**
     * Fires on the 1→0 transition — the last currently-running
     * `PinnedActivity` for this block just ended. No-op if already null.
     */
    | { type: "AttachedTaskCleared" }

    // ── Stale-`--resume` reconnect axis
    // (STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2) ──
    /**
     * `mps.EVENT_AGENT_RESUME_RETRY`'s `{status:"retrying"}` landed: a stale `--resume` was
     * detected and a retry is about to fire. Idempotent: a cascaded second retry must not push
     * `startedAt` later.
     */
    | { type: "ResumeRetryStarted"; at: number }
    /**
     * `{status:"resolved"}` landed — the retry's outcome (Fresh or Resumed)
     * is now known. No-op if already null.
     */
    | { type: "ResumeRetryResolved" }

    // ── Registry-derived attached-task floor (Phase C of
    // SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md): see `registryAttachedTaskSince`'s
    // doc for why it is a separate axis rather than AttachedTaskObserved/Cleared. ──
    /** Sets `registryAttachedTaskSince` verbatim (idempotent set, not an
     * edge-triggered toggle like AttachedTaskObserved — the registry's own
     * earliest-running-task computation already handles the "already
     * observed" case, so the reducer doesn't need its own no-op guard). */
    | { type: "RegistryAttachedTaskObserved"; at: number }
    /** Clears `registryAttachedTaskSince` back to null. */
    | { type: "RegistryAttachedTaskCleared" }
    ;

export type AgentPaneEvent =
    | { type: "init-ready" }
    | { type: "init-failed"; reason: string }
    | { type: "stream-subscribed"; at: number }
    | { type: "stream-unsubscribed"; at: number }
    /**
     * The stream tore down while a turn was in flight; emitted alongside `stream-unsubscribed`,
     * only on the working-set → Disconnected transition (not on idle unsubscribes), so the view can
     * drive a "Disconnected — reconnecting…" banner.
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
          /** The WARNING bar (STUCK_THRESHOLD_MS). Crossing it is cosmetic. */
          thresholdMs: number;
          /**
           * The bar that would force this turn back to Idle: always larger than `thresholdMs`, and
           * larger still under a rate-limit backoff.
           */
          recoverThresholdMs: number;
      }
    | {
          /**
           * The watchdog force-recovered a hung `Streaming` turn to `Idle` after `idleSinceMs` ≥
           * `LIVENESS_RECOVERY_MS` (its `session_end` was never observed). For telemetry; the phase
           * change is the real effect.
           */
          type: "working-recovered";
          idleSinceMs: number;
          thresholdMs: number;
      }
    | { type: "turn-started"; at: number }
    /**
     * `ReconcileTurnActive` promoted the phase to `Streaming` (from the mount-default `Idle` or a
     * settled `Done.completed` episode). For diagnostics: distinguishes this from a user-initiated
     * `turn-started`.
     */
    | { type: "turn-active-reconciled" }
    /**
     * `ReconcileTurnActive` demoted a stuck `Streaming` phase to `Idle`: the backend reported the
     * turn over while the frontend never saw `session_end`. For telemetry; the phase change is the
     * real effect.
     */
    | { type: "turn-inactive-reconciled"; at: number }
    /**
     * `ReconcileContextFromHistory` seeded `context` from the restored
     * transcript's last main-agent call at mount. Surfaced for diagnostics only.
     */
    | { type: "context-reconciled-at-mount"; tokens: number }
    /**
     * A reading that can't be true (more tokens than its window, or than any
     * known window with none) was refused or will not display. Logged by the
     * pane store; the meter shows nothing rather than a wrong number. See
     * context-reading.ts `implausibleReason`.
     */
    | {
          type: "context-reading-rejected";
          tokens: number;
          window: number | null;
          model: string | null;
          source: ContextSource;
          reason: string;
      }
    /**
     * An accepted prompt was larger than the window Claude Code reported for its model, so the
     * reported window can't be right; the meter uses the next known tier above the prompt. Logged
     * once per model by the pane store (context-reading.ts `refutedReportedWindow`).
     */
    | { type: "context-window-refuted"; model: string; reported: number; tokens: number; window: number }
    | {
          /**
           * A turn finished and the phase became `Done`. Carries `outcome` directly so consumers
           * (notification sounds, telemetry) needn't snapshot the slot
           * (SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §3.2).
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
     * `source: "real"`: an authoritative `CompactionBoundary` event (`trigger` / `durationMs`
     * present). `source: "heuristic"`: inferred from a ≥50% drop in `TokensIn` (both absent).
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
          /** `source: "real"` only — see `CompactionBoundary` command's doc comment. */
          boundaryUuid?: string | null;
      }
    /** `CompactionStarted` landed — compaction is in progress. */
    | { type: "compaction-started"; trigger: "manual" | "auto" }
    /**
     * `CompactionStarted` arrived before `turnPhase` was confirmed working
     * — buffered onto `state.pendingCompactionPing` rather than dropped.
     * See `SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md`.
     */
    | { type: "compaction-started-buffered"; trigger: "manual" | "auto" }
    | { type: "provider-waiting"; reason: "rate_limited" }
    /**
     * A failure was observed and recorded on `state.failure`. `turnWasEnded` is true iff the turn
     * was actually working at the time (for diagnostics: a genuine mid-turn failure vs a stray late
     * event after idle).
     */
    | { type: "failure-observed"; code: AgentFailure["code"]; turnWasEnded: boolean }
    /** `state.failure` was cleared (explicit dismiss, or implicitly by TurnStart). */
    | { type: "failure-cleared"; reason?: FailureClearReason }
    | { type: "stop-requested"; at: number }
    | { type: "stop-failed" }
    /**
     * Reducer signal: a turn has just entered Interrupting. The dispatch layer starts the
     * `INTERRUPT_TIMEOUT_MS` timer and fires `InterruptTimeoutElapsed` on expiry, gated by a shared
     * cancel flag so a later `RequestStop` doesn't double-fire. `deadlineMs` is `at +
     * INTERRUPT_TIMEOUT_MS`.
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
     * Reducer signal: a turn has just entered Submitting. The dispatch layer starts the
     * `SUBMIT_TIMEOUT_MS` timer and fires `SubmitTimeoutElapsed` on expiry, gated by a shared
     * cancel flag so a promotion to Streaming cancels it first. `deadlineMs` is `at +
     * SUBMIT_TIMEOUT_MS`.
     */
    | {
          type: "schedule-submit-timeout";
          deadlineMs: number;
      }
    /**
     * Reducer signal: the bounded `Submitting → Done.{errored}` force transition fired because no
     * stream activity arrived within `SUBMIT_TIMEOUT_MS`. Carries the firing wall-clock for audit
     * (spec §8, #728 gap 2).
     */
    | { type: "submit-timed-out"; at: number }
    | { type: "pending-flush-started"; id: string }
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
     * Emitted when the pane enters the waiting-for-input state (turn completed with a question,
     * composer empty); the sound service starts the looping ambient tone on it
     * (SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md §6.3).
     */
    | {
          type: "waiting-for-input";
          /** Text of the first pending question, when known. Consumed by the OS
           *  notification bridge, which forwards it to the srv router for
           *  redaction (SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §9.2). */
          question?: string;
          /** How many questions the pending call asks — the toast adds
           *  " (+N more)" when it's more than one. */
          questionCount?: number;
      }
    /**
     * Emitted when the waiting state ends — user submitted or started
     * typing, pane closed, or the 5-minute safety cutoff fired.
     */
    | { type: "waiting-ended"; reason: "submitted" | "typing" | "closed" }
    /** `AttachedTaskObserved` set `state.attachedTask` (0→1 edge). */
    | { type: "attached-task-observed"; at: number }
    /** `AttachedTaskCleared` cleared `state.attachedTask` (1→0 edge). */
    | { type: "attached-task-cleared" }
    /** `RegistryAttachedTaskObserved` set `state.registryAttachedTaskSince`. */
    | { type: "registry-attached-task-observed"; at: number }
    /** `RegistryAttachedTaskCleared` cleared `state.registryAttachedTaskSince`. */
    | { type: "registry-attached-task-cleared" }
    /** `ResumeRetryStarted` set `state.reconnecting` (0→1 edge). */
    | { type: "resume-retry-started"; at: number }
    /** `ResumeRetryResolved` cleared `state.reconnecting` (1→0 edge). */
    | { type: "resume-retry-resolved" };

export interface ReducerResult {
    state: AgentPaneState;
    events: AgentPaneEvent[];
}

/**
 * Stuck-stream threshold (#728 gap 3): a subscribed stream with no observable event (subscribe,
 * flush, tool, tokens) for this many ms is flagged. 45s covers the typical Claude/Codex idle window
 * between thinking and tool calls in long reasoning runs without false positives.
 */
export const STUCK_THRESHOLD_MS = 45_000;

/**
 * Liveness-recovery threshold: a `Streaming` turn with no observable activity, no tool running and
 * not rate-limited for this long is treated as hung (its `session_end` was never observed). 3 min
 * is well past `STUCK_THRESHOLD_MS` (45s), so a quiet reasoning run is never mistaken for a hang.
 */
export const LIVENESS_RECOVERY_MS = 180_000;

/**
 * Bounded `Interrupting → Done.{interrupted}` window: after `RequestStop` fires SIGINT, if neither
 * `TurnEnd` nor `StreamUnsubscribe` lands within this many ms the reducer force-transitions on
 * `InterruptTimeoutElapsed`.
 */
export const INTERRUPT_TIMEOUT_MS = 5_000;

/** A Streaming phase's rate-limit wait, cleared: the reason, its Retry-After
 *  and the deadline the countdown reads, all at once. */
export const NO_RATE_LIMIT_WAIT = { waitingReason: undefined, retryAfterMs: undefined, retryAtMs: undefined } as const;

/**
 * Bounded `Submitting → Done.{errored}` window: with no stream activity within this many ms of
 * `TurnStart`, the reducer force-transitions on `SubmitTimeoutElapsed`. 30 s is the outer bound for
 * a backend ack (handshake, agent spawn, first token); the streaming watchdog covers the longer
 * thinking idle case.
 */
export const SUBMIT_TIMEOUT_MS = 30_000;

/**
 * Suppression window for the `TokensIn` ≥50%-drop compaction heuristic after a REAL
 * `CompactionBoundary`: a backstop against an ordering surprise or a `postTokens` that doesn't
 * match the next `TokensIn.input`. 2 minutes covers one turn's round trip without suppressing a
 * LATER genuine compaction.
 */
export const COMPACTION_HEURISTIC_SUPPRESS_MS = 120_000;
