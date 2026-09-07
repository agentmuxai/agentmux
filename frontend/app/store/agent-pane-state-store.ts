// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent pane state store — slice #4 of the frontend reducer roadmap.
 * Bundles the per-pane lifecycle/turn/tool/tokens/stop/pending state.
 *
 * Pattern matches agent-document-store.ts: per-blockId slot, throw on
 * unregistered dispatch. Conventions §4–§5
 * (frontend-reducer-conventions-2026-05-03.md).
 *
 * The reactive read side lives HERE (see `AgentPaneView`), not in a
 * view-owned mirror. Before A6 of issue #1549 the view created 17 Solid
 * signals (`createAgentAtoms`), handed their setters in through an
 * `AgentPaneProjections` interface, and this store wrote each changed field
 * through the matching setter — adding a reducer field meant editing the
 * state type, `initialState`, the projections interface, `createAgentAtoms`,
 * and the wiring block in `agent-view.tsx`, and forgetting one was silent.
 * The view could also write an atom directly, bypassing the reducer that
 * owned the field (it did, for `detailsOpen`).
 */

import { type Accessor, batch, createSignal, type Setter } from "solid-js";

import { update } from "./agent-pane-state/reducer";
import {
    AgentPaneCommand,
    AgentPaneEvent,
    AgentPaneState,
    initialState,
    workingFromPhase,
} from "./agent-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * Reactive, read-only view of a pane's reducer state — one Solid signal per
 * top-level `AgentPaneState` field, generated from the state's own keys.
 * Reads are plain property access (`view.turnPhase.kind`) and track like
 * any signal read.
 *
 * Semantics are exactly the old per-field projections': each field is its
 * own signal with referential (`===`) equality, so a dispatch that leaves a
 * field's object untouched does not notify that field's readers. The
 * reducer keeps treating state as immutable (fresh objects on change), which
 * is what makes per-field identity a valid change signal.
 *
 * Why not `createStore` + `reconcile` (the `launch-flow-store` pattern):
 * `reconcile` diffs INTO the store's existing object tree in place. That tree
 * would come to alias the reducer's own state objects, so a later reconcile
 * would mutate a `prev` the diagnostics in `dispatch()` still compare
 * against — and any `snapshot()` a caller was holding. Per-field signals
 * never mutate anything the reducer produced.
 */
export type AgentPaneView = { readonly [K in keyof AgentPaneState]: AgentPaneState[K] };

type FieldSignals = {
    [K in keyof AgentPaneState]: [Accessor<AgentPaneState[K]>, Setter<AgentPaneState[K]>];
};

function createFieldSignals(init: AgentPaneState): { signals: FieldSignals; view: AgentPaneView } {
    // Built key-by-key, so the per-key generic type can't be expressed to
    // TS inside the loop (it would need the intersection of every field's
    // signal type). The erased-type map is cast once at the end; the
    // public `FieldSignals` / `AgentPaneView` shapes are what callers see.
    const signals: Record<string, [Accessor<unknown>, Setter<unknown>]> = {};
    const view: Record<string, unknown> = {};
    for (const key of Object.keys(init) as (keyof AgentPaneState)[]) {
        // createSignal's default equality is `===` — the same test the old
        // `proj()` helper applied before calling a setter.
        const sig = createSignal<unknown>(init[key]);
        signals[key] = sig;
        Object.defineProperty(view, key, { get: sig[0], enumerable: true });
    }
    return { signals: signals as unknown as FieldSignals, view: Object.freeze(view) as AgentPaneView };
}

interface Slot {
    state: AgentPaneState;
    signals: FieldSignals;
    view: AgentPaneView;
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
 * function are limited to single-store unit tests. PR-3 of the cascade
 * follow-up sequence — see agent-pane-registration.ts for rationale +
 * Option A/B discussion.
 */
export function registerPane(blockId: string, agentId: string): void {
    const state = initialState(agentId);
    const { signals, view } = createFieldSignals(state);
    slots.set(blockId, { state, signals, view, stuckLogged: false, watchdogTickCount: 0 });
}

/**
 * The pane's reactive state view (see `AgentPaneView`), or `null` if the
 * pane is not registered. Production code reaches this through
 * `AgentPaneModel.state` (agent-pane-registration.ts hands the model the
 * view at register time); this export is for tests and diagnostics.
 */
export function paneView(blockId: string): AgentPaneView | null {
    return slots.get(blockId)?.view ?? null;
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

    // Publish changed fields to the reactive view — one signal write per
    // field whose value identity changed (the same referential test the old
    // per-field projection setters used), inside one `batch` so a reader of
    // two fields sees them move together and effects run once per dispatch.
    //
    // Cascade detection: docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
    // A reactive subscriber of one of these fields can synchronously unmount
    // the pane (call `unregisterPane`) when the batch flushes. Because every
    // write lands inside one `batch()`, subscriber effects run only after all
    // of them — so, unlike the old per-setter loop, this cannot single out
    // WHICH field's reader disposed the pane. It records the set of fields
    // this dispatch changed (the reader belongs to one of them), which is what
    // the warning below names. The next dispatch in the caller's frame will
    // throw.
    const next = slot.state;
    const changed: string[] = [];
    batch(() => {
        for (const key of Object.keys(slot.signals) as (keyof AgentPaneState)[]) {
            if (prev[key] === next[key]) continue;
            changed.push(key);
            // Always pass a thunk: a bare value that happened to be a function
            // would be treated as an updater by Solid's setter.
            (slot.signals[key][1] as Setter<unknown>)(() => next[key]);
        }
    });
    if (import.meta.env.DEV) {
        // A field the reducer produced but `initialState()` didn't declare has
        // no signal and would silently never be reactive. Every field of
        // `AgentPaneState` is required, so this only fires if someone adds an
        // optional field and forgets `initialState` — the one place left.
        for (const key of Object.keys(next)) {
            if (!(key in slot.signals)) {
                console.warn(
                    `[agent-pane-state] field '${key}' is on the reducer state but missing from initialState() — it will not be reactive`,
                );
            }
        }
    }
    if (changed.length > 0 && !slots.has(blockId)) {
        console.warn(
            `[agent-pane-state] CASCADE_DETECTED: a subscriber of '${changed.join("'/'")}' disposed pane mid-dispatch ` +
            `(cmd=${command.type}, blockId=${blockId.slice(0, 7)}, source=${source}). ` +
            `A reactive reader of one of those fields unmounted the pane during dispatch. ` +
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
        } else if (
            ev.type === "attached-task-observed" ||
            ev.type === "attached-task-cleared" ||
            ev.type === "registry-attached-task-observed" ||
            ev.type === "registry-attached-task-cleared"
        ) {
            // The attached-task axis (turnPhase's sibling for "≥1 agent-
            // declared long-running task is live", see
            // SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md) had zero logging
            // of its own — every OTHER axis this file logs (turnPhase
            // transitions, the watchdog's reasoning) has a `[wave-turn]`
            // line, but this one was silent, which matters specifically for
            // the "pane reads Worked/Idle while a long-running process is
            // still attached" class of report: without this, there was no
            // way to confirm from `muxlog phases` alone whether the axis
            // correctly tracked the attached task through that window, or
            // never observed it in the first place. Logs `turnPhase.kind`
            // alongside so a reader can directly see the Done/Idle-while-
            // attached shape land, or catch the axis itself failing to
            // observe/clear at the expected moment.
            // See docs/specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md.
            console.info(
                "[wave-turn]",
                `pane=${blockId.slice(0, 7)}`,
                `attachedTask: ${ev.type}`,
                `phase=${slot.state.turnPhase.kind}`,
                `attachedTask=${slot.state.attachedTask ? `since=${slot.state.attachedTask.since}` : "null"}`,
                `registryAttachedTaskSince=${slot.state.registryAttachedTaskSince ?? "null"}`,
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

/** Snapshot — diagnostics + tests only. Non-reactive; for reactive reads use `paneView`. */
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
