// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent pane state store — slice #4 of the frontend reducer roadmap.
 * Bundles the per-pane lifecycle/turn/tool/tokens/stop/pending atoms.
 *
 * Pattern matches agent-document-store.ts: per-blockId slot, atoms as
 * write-only projections, throw on unregistered dispatch. Conventions
 * §4–§5 (frontend-reducer-conventions-2026-05-03.md).
 */

import type { PendingMessage } from "../view/agent/state";
import type {
    SessionStats,
    StreamingState,
    TurnTokens,
} from "../view/agent/types";
import { update } from "./agent-pane-state/reducer";
import {
    AgentPaneCommand,
    AgentPaneEvent,
    AgentPaneState,
    type AttachedTaskState,
    type CompactionState,
    type InitPhase,
    initialState,
    type PaneFailure,
    type TurnPhase,
    workingFromPhase,
} from "./agent-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * The set of projection setters the slot writes to. Each one corresponds
 * to a pre-existing per-pane Solid signal in `createAgentAtoms`. Readers
 * keep using the existing accessors; only writes are routed through this
 * store.
 *
 * PR G dropped the `turnActive` and `stopping` setters — the legacy
 * fields they backed are gone. The view binds its "working" animation
 * to `workingFromPhase(turnPhase)` and its "Stopping…" label to
 * `turnPhase.kind === "Interrupting"`.
 */
export interface AgentPaneProjections {
    streaming: (next: StreamingState) => void;
    sessionStats: (next: SessionStats | null) => void;
    /** Cumulative session totals — see SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md. */
    sessionTotals: (next: SessionStats | null) => void;
    currentTool: (next: string | null) => void;
    turnTokens: (next: TurnTokens | null) => void;
    pending: (next: PendingMessage[]) => void;
    /** Init phase — drives the "Loading history…" overlay (issue #728 gap 1). */
    initPhase?: (next: InitPhase) => void;
    /**
     * Single-source-of-truth turn phase. Since PR G this is the only
     * working/stopping signal the view binds to (via
     * `workingFromPhase(turnPhase)` and `turnPhase.kind === "Interrupting"`).
     * Optional so existing callers (and the cascade-store test's no-op
     * projection) keep compiling.
     */
    turnPhase?: (next: TurnPhase) => void;
    /**
     * Composer details panel — open/closed. Reducer-owned (PR #1068).
     * Drives the chevron orientation in the composer strip + the
     * conditional render of the details panel. Optional for back-compat
     * with existing test projections.
     */
    detailsOpen?: (next: boolean) => void;
    /**
     * First significant argument of the active tool call (file path for
     * read/write, command string for bash, etc.). Cleared on ToolEnd.
     * Drives enriched AgentWorkingRow display.
     */
    currentToolArg?: (next: string | null) => void;
    /**
     * Current input-token count as of the last message_start — equals the
     * total context fill (all conversation history) sent to the model.
     * Driven by the same TokensIn command as turnTokens.input; fires once
     * per turn at message_start. Persists through TurnEnd so the bar stays
     * visible between turns. Clears only on TurnReset (session wipe).
     */
    contextTokens?: (next: number | null) => void;
    /** Learned context-window size for the current model (null → view uses the
     *  provider's static fallback). Driven by TokensIn alongside contextTokens. */
    contextWindow?: (next: number | null) => void;
    /**
     * Active classified failure for this pane, or null. Reducer-owned
     * (see `AgentPaneState.failure` / `FailureObserved` / `FailureCleared`).
     * See SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
     */
    failure?: (next: PaneFailure | null) => void;
    /**
     * Live "compaction in progress" state, or null. Reducer-owned (see
     * `AgentPaneState.compacting` / `CompactionStarted` / `CompactionBoundary`).
     * Drives the "Compacting…" status chip + elapsed counter. See
     * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md.
     */
    compacting?: (next: CompactionState | null) => void;
    /**
     * Live "≥1 agent-declared long-running activity attached" state, or
     * null. Reducer-owned (see `AgentPaneState.attachedTask` /
     * `AttachedTaskObserved` / `AttachedTaskCleared`). Drives the
     * "Running…" footer status once `turnPhase` is otherwise idle — the
     * fix for the Agent1 "stuck Working for 12h" retro
     * (retro-persistent-agent-working-status-stuck-2026-07-16.md). See
     * docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md.
     */
    attachedTask?: (next: AttachedTaskState | null) => void;
    /**
     * Live registry-derived attached-task floor, or null. Reducer-owned
     * (see `AgentPaneState.registryAttachedTaskSince` /
     * `RegistryAttachedTaskObserved` / `RegistryAttachedTaskCleared`) — a
     * SEPARATE axis from `attachedTask` above, combined with it by
     * `agent-view.tsx`'s attached-task effect rather than merged into it.
     * See docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md.
     */
    registryAttachedTaskSince?: (next: number | null) => void;
}

interface Slot {
    state: AgentPaneState;
    proj: AgentPaneProjections;
    // Edge-trigger for the `[wave-turn]` stream-stuck watchdog line — logs
    // once per stall episode (on the first threshold crossing) instead of
    // every 5s watchdog tick for as long as the stall lasts. Reset on every
    // turnPhase.kind transition, see dispatch().
    stuckLogged: boolean;
    // Counts `StreamWatchdogTick` dispatches for this pane since it was
    // registered. Every `WATCHDOG_HEARTBEAT_EVERY_N_TICKS`th tick gets a
    // `[wave-turn] watchdog: tick` line regardless of whether the reducer
    // found anything stuck — the edge-triggered `stream-stuck`/
    // `working-recovered` lines below only fire when something is WRONG,
    // so a genuinely-dead watchdog interval (never dispatching this
    // command at all) produces total silence, indistinguishable from "the
    // watchdog is fine and nothing needed recovering." That ambiguity is
    // exactly what made docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md's
    // root-cause reasoning (§4) an inference from absence rather than a
    // direct fact ("if it were ticking we'd see a line — we don't").
    // See docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md.
    watchdogTickCount: number;
}

// One heartbeat line per this many `StreamWatchdogTick` dispatches (5s
// interval per useTurnLifecycle.ts's WATCHDOG_INTERVAL_MS) — ~60s cadence.
// Cheap proof-of-life without flooding a long-running pane's log with a
// line every 5s.
const WATCHDOG_HEARTBEAT_EVERY_N_TICKS = 12;

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: AgentPaneEvent) => void;
let eventSink: EventSink = (blockId, event) => {
    if (event.type === "turn-start-suppressed") {
        console.warn(
            `[agent-pane-state] turn-start suppressed for ${blockId.slice(0, 7)}: ${event.reason}`,
        );
    }
};

export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Additional listeners that receive a copy of every emitted event
 * alongside the single `eventSink` above. Multiple subscribers are
 * supported — used by the sound-notifications subsystem (see
 * SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.4 Path B) without
 * displacing the existing single-sink consumers in
 * `browser-model.ts` / `editor-model.ts`. Each listener is invoked
 * in a try/catch so a throwing subscriber cannot poison the others
 * or the primary sink.
 */
const extraListeners = new Set<EventSink>();

export function addEventListener(sink: EventSink): () => void {
    extraListeners.add(sink);
    return () => {
        extraListeners.delete(sink);
    };
}

/** Test helper — wipe all multicast listeners. Never call in production. */
export function __resetListeners(): void {
    extraListeners.clear();
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the component body, before
 * any hook can dispatch. Re-registering a blockId resets the state cell
 * to initialState (useful for hot-reload).
 *
 * @internal — production callers MUST use `registerPane` from
 * `agent-pane-registration.ts` so the pane is registered atomically
 * across BOTH stores (document + pane-state). Direct callers of this
 * function are limited to single-store unit tests (cascade-detection
 * scenarios that need a custom single-store projection). PR-3 of the
 * cascade follow-up sequence — see agent-pane-registration.ts for
 * rationale + Option A/B discussion.
 */
export function registerPane(
    blockId: string,
    agentId: string,
    proj: AgentPaneProjections,
): void {
    slots.set(blockId, { state: initialState(agentId), proj, stuckLogged: false, watchdogTickCount: 0 });
}

/**
 * @internal — see `registerPane` above. Production code uses
 * `unregisterPane` from `agent-pane-registration.ts`.
 */
export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops would
 * defeat the point of the reducer (same rule as agent-document-store).
 */
export function dispatch(
    blockId: string,
    command: AgentPaneCommand,
    source: CommandSource = "system",
): AgentPaneEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[agent-pane-state] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the component body.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    // [wave-turn] diagnostics — mirrors app-init.ts's `[wave-title]` line
    // (tail with `muxlog host '\[fe\] \[wave-turn\]'`). Before this, an
    // agent debugging "why does this pane say Working" had nothing to
    // grep: the reducer is a pure function (zero logging of its own) and
    // `dispatch()`'s eventSink only ever logged `turn-start-suppressed`.
    // Every other transition, and the watchdog's own reasoning for
    // whether it recovered a hung turn, was silently discarded. See
    // docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md §3.
    //
    // Gate on `.kind`, not object identity — `StreamFlushObserved` returns a
    // fresh `turnPhase` object on every RAF-batched flush (up to ~60/sec
    // while streaming) even when `kind` stays "Streaming" (reagentx P1 on
    // PR #2321: referential inequality flooded muxlog with a line per frame
    // for the whole duration of every response).
    //
    // `console.info`, not `.debug` — the host's default EnvFilter is "info"
    // (no RUST_LOG set), which silently drops `debug!`/console.debug lines.
    // Logging at info keeps this discoverable in a default run, which is the
    // whole point of a post-incident self-diagnostic line (codex P1 on PR
    // #2321). Now that the flood above is fixed, real transitions are rare
    // enough (a handful per turn) that info-level volume is fine.
    // Any refresh of the liveness clock (lastEventMs) — not just a kind
    // change — means a new stall episode can happen and should be logged
    // again. `bumpEvent`-driven tool/token activity refreshes lastEventMs
    // while `kind` stays "Streaming" throughout, so gating the reset on
    // `.kind` alone silently dropped the second of two stalls inside one
    // continuous Streaming phase (reagentx P2 re-review on PR #2321).
    if (prev.lastEventMs !== slot.state.lastEventMs) {
        slot.stuckLogged = false;
    }
    if (prev.turnPhase.kind !== slot.state.turnPhase.kind) {
        // NOTE: an earlier version of this line auto-tagged every
        // `StreamFlushObserved` promotion from a non-`Submitting` phase as
        // `(stray)`, on the theory that only `Submitting → Streaming` is
        // the "normal" hand-off. reagent P1 on PR #2653 caught that this is
        // wrong: reducer.ts's `StreamFlushObserved` arm documents BOTH
        // `Idle`/`Disconnected` re-promotion (a legitimate stream drop +
        // resubscribe, e.g. an agent respawn mid-stall) AND `Done.completed`
        // re-promotion (session_end fires after every model API round, so
        // this is the normal shape of a multi-round tool continuation) as
        // intentional, non-anomalous cases — NOT the rare genuine-anomaly
        // shape docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md
        // found (§3). A blanket "not Submitting" heuristic mislabels the
        // common healthy case, drowning out the rare real one — the exact
        // opposite of this feature's purpose. There is no reliable way to
        // tell the two apart from this transition alone (both look
        // identical: `Done → Streaming cmd=StreamFlushObserved`); the Aug
        // 14 report's own strayness conclusion required EXTERNAL context
        // (the backend's independent `[health] active:false` signal, and
        // that literally nothing else ever arrived afterward) that isn't
        // available at dispatch time. The already-reliable signal for "this
        // promotion never resolved" is the existing edge-triggered
        // `stream-stuck`/`working-recovered` lines below (elapsed-time-
        // based, not shape-based) plus the watchdog heartbeat above — no
        // auto-tag needed on this line; a reader of `muxlog phases` can
        // already see the raw `X → Y` transition and judge it themselves
        // with the merged fe+srv context the recipe exists to provide. See
        // docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md's
        // Implementation notes for the full account.
        console.info(
            "[wave-turn]",
            `pane=${blockId.slice(0, 7)}`,
            `${prev.turnPhase.kind} → ${slot.state.turnPhase.kind}`,
            `cmd=${command.type}`,
            `toolsActive=${slot.state.turnPhase.kind === "Streaming" ? slot.state.turnPhase.toolsActive : "-"}`,
            `currentTool=${slot.state.currentTool ?? "-"}`,
        );
    }

    // Periodic watchdog-liveness heartbeat — see WATCHDOG_HEARTBEAT_EVERY_N_TICKS's
    // doc comment on `Slot` for why this exists alongside the edge-triggered
    // stream-stuck/working-recovered lines below rather than replacing them.
    if (command.type === "StreamWatchdogTick") {
        slot.watchdogTickCount++;
        if (slot.watchdogTickCount % WATCHDOG_HEARTBEAT_EVERY_N_TICKS === 0) {
            console.info(
                "[wave-turn]",
                `pane=${blockId.slice(0, 7)}`,
                `watchdog: tick #${slot.watchdogTickCount} — alive, phase=${slot.state.turnPhase.kind}`,
            );
        }
    }

    // Project changes — only call setters for fields that actually
    // changed (referential equality). Avoids redundant signal writes.
    // Per-setter cascade detection: docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
    // A reactive subscriber on the atom backing one of these setters can
    // synchronously unmount the pane (call `unregisterPane`) inside the
    // setter call. Capture which setter triggers the dispose — the next
    // dispatch in the caller's frame will throw and the log line below
    // pinpoints the cause.
    let cascadeSetter: string | null = null;
    const proj = <T>(name: string, prev: T, next: T, set: ((v: T) => void) | undefined): void => {
        if (prev === next) return;
        set?.(next);
        if (cascadeSetter == null && !slots.has(blockId)) cascadeSetter = name;
    };
    proj("streaming", prev.streaming, slot.state.streaming, slot.proj.streaming);
    proj("sessionStats", prev.sessionStats, slot.state.sessionStats, slot.proj.sessionStats);
    proj("sessionTotals", prev.sessionTotals, slot.state.sessionTotals, slot.proj.sessionTotals);
    proj("currentTool", prev.currentTool, slot.state.currentTool, slot.proj.currentTool);
    proj("turnTokens", prev.turnTokens, slot.state.turnTokens, slot.proj.turnTokens);
    proj("contextTokens",
        prev.lastContextTokens ?? null,
        slot.state.lastContextTokens ?? null,
        slot.proj.contextTokens);
    proj("contextWindow",
        prev.lastContextWindow ?? null,
        slot.state.lastContextWindow ?? null,
        slot.proj.contextWindow);
    proj("pending", prev.pending, slot.state.pending, slot.proj.pending);
    proj("initPhase", prev.initPhase, slot.state.initPhase, slot.proj.initPhase);
    proj("turnPhase", prev.turnPhase, slot.state.turnPhase, slot.proj.turnPhase);
    proj("detailsOpen", prev.detailsOpen, slot.state.detailsOpen, slot.proj.detailsOpen);
    proj("currentToolArg", prev.currentToolArg, slot.state.currentToolArg, slot.proj.currentToolArg);
    proj("failure", prev.failure, slot.state.failure, slot.proj.failure);
    proj("compacting", prev.compacting, slot.state.compacting, slot.proj.compacting);
    proj("attachedTask", prev.attachedTask, slot.state.attachedTask, slot.proj.attachedTask);
    proj("registryAttachedTaskSince", prev.registryAttachedTaskSince, slot.state.registryAttachedTaskSince, slot.proj.registryAttachedTaskSince);

    if (cascadeSetter != null) {
        console.warn(
            `[agent-pane-state] CASCADE_DETECTED: '${cascadeSetter}' setter disposed pane mid-dispatch ` +
            `(cmd=${command.type}, blockId=${blockId.slice(0, 7)}, source=${source}). ` +
            `A reactive subscriber on the '${cascadeSetter}' atom unmounted the pane during dispatch. ` +
            `Subsequent dispatches in the same callback will throw.`,
        );
    }

    for (const ev of result.events) {
        // Watchdog reasoning — the reducer already computes exactly why it
        // did or didn't recover a hung turn (reducer.ts's StreamWatchdogTick
        // branch); surface it instead of discarding it. `EXEMPT` on
        // stream-stuck is the single highest-value line for self-diagnosing
        // a "Working for no reason" report: it names the tool that's
        // keeping the pane from ever being force-recovered.
        //
        // `stream-stuck` fires on every 5s watchdog tick once idle passes
        // STUCK_THRESHOLD_MS — including for panes sitting at Done/Idle,
        // since `lastEventMs` isn't cleared on those transitions (codex P2
        // on PR #2321). Gate on `workingFromPhase` so this only fires for
        // panes actually showing "Working", and edge-trigger via
        // `slot.stuckLogged` so a genuine stall logs once (on the first
        // threshold crossing), not every tick for the rest of the stall.
        if (ev.type === "stream-stuck") {
            const p = slot.state.turnPhase;
            if (workingFromPhase(p) && !slot.stuckLogged) {
                slot.stuckLogged = true;
                const exempt = p.kind === "Streaming" && p.toolsActive > 0;
                console.info(
                    "[wave-turn]",
                    `pane=${blockId.slice(0, 7)}`,
                    `watchdog: no recovery — idleSinceMs=${ev.idleSinceMs} thresholdMs=${ev.thresholdMs}`,
                    exempt ? `EXEMPT toolsActive=${p.toolsActive} currentTool=${slot.state.currentTool ?? "?"}` : "",
                );
            }
        } else if (ev.type === "working-recovered") {
            slot.stuckLogged = false;
            console.info(
                "[wave-turn]",
                `pane=${blockId.slice(0, 7)}`,
                `watchdog: FIRED — force-recovered to Idle, idleSinceMs=${ev.idleSinceMs}`,
            );
        }
        eventSink(blockId, ev);
        for (const l of extraListeners) {
            try {
                l(blockId, ev);
            } catch (e) {
                console.warn(
                    `[agent-pane-state] multicast listener threw (cmd=${command.type}, ev=${ev.type})`,
                    e,
                );
            }
        }
    }
    recordDispatch({
        slice: "agent-pane-state",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });
    return result.events;
}

/**
 * Soft-dispatch variant. Returns an empty event array if the slot is
 * already gone, instead of throwing. Use ONLY from async contexts
 * (RAF / setTimeout / setInterval / await continuations / subscription
 * handlers) where a normal dispatch can race against the pane's
 * onCleanup unregistering the slot — see
 * docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md §6.1 option B.
 *
 * Synchronous component-body dispatches MUST continue to use `dispatch`
 * — a missing slot there is a registration-order bug and the throw is
 * the right signal.
 */
export function dispatchIfRegistered(
    blockId: string,
    command: AgentPaneCommand,
    source: CommandSource = "system",
): AgentPaneEvent[] {
    if (!slots.has(blockId)) return [];
    return dispatch(blockId, command, source);
}

/**
 * Fire a synthetic event directly to the multicast listeners (e.g. the sound
 * service) without going through the reducer. Used by components that have
 * their own reactive state (e.g. `pendingQuestions` in agent-view) and need
 * to drive audio without coupling the reducer to document-layer detail.
 */
export function fireEvent(blockId: string, event: AgentPaneEvent): void {
    eventSink(blockId, event);
    for (const l of extraListeners) {
        try { l(blockId, event); } catch { /* isolate */ }
    }
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): AgentPaneState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper. */
export function __resetAllSlots(): void {
    slots.clear();
}

/**
 * Returns a map of definition_id → blockId for all currently-open agent panes.
 * Used by AgentPicker to detect when a definition is already open so it can
 * show the fork prompt instead of silently reattaching.
 */
export function getOpenDefinitionMap(): Map<string, string> {
    const result = new Map<string, string>();
    for (const [blockId, slot] of slots) {
        const defId = slot.state.streaming.agentId;
        if (defId) result.set(defId, blockId);
    }
    return result;
}

export type { AgentPaneCommand, AgentPaneEvent, AgentPaneState };
