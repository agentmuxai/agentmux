// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.6 — renderer multi-source dispatcher with saga buffering.
//
// Shared infrastructure for `srv-events.ts` and `launcher-events.ts`.
// Same per-source state machine drives both pipes. Phase F will add
// a third (`host-events.ts`) when host events cross IPC; this module
// is source-agnostic so adding the third bucket is a constructor
// call away.
//
// **Three jobs:**
//
// 1. **Per-source version monotonicity.** Detect dropped events:
//    each source's reducer emits a strictly-increasing `version`,
//    so any gap means an event was lost on the wire. We log the gap
//    and increment a `droppedCount`; recovery (force-push) is the
//    consumer's responsibility — see "Force-push protocol" below.
//
// 2. **Saga buffering.** Between `SagaStarted { saga_id }` and the
//    matching `SagaCompleted` / `SagaFailed`, all events from the
//    same source are buffered. On terminal, the buffer is flushed
//    inside `solid.batch(...)` so SolidJS-effect consumers (atom
//    routers) see the saga as a single tick — no half-applied flicker.
//    Per-event callback subscribers still receive every event
//    individually, in order — they need that to drive correct atom
//    state transitions; the batch only collapses the *signal* updates.
//
// 3. **Stale event drop.** If an event arrives with `version` <=
//    last seen, it's a replay or out-of-order delivery. Drop it.
//
// **Saga correlation today is temporal.** Per-step events do NOT
// carry `saga_id` (per-Event threading was deferred — see
// `docs/retro/reducer-architecture-gaps-2026-05-01.md` §3). Sagas
// run serially in the srv coordinator, so events between
// `SagaStarted` and the terminal are reliably *this* saga's. If
// concurrent sagas land in a future PR, this module will need
// per-event `saga_id` tagging.
//
// **Resync ordering contract.** Snapshot replay must complete before
// live-event application begins. The host-side bridge guarantees
// this by holding the websocket reader paused until the snapshot
// reply arrives; this module does not enforce — it would require
// the snapshot to be fed through the same pipe, which is not the
// design.
//
// **Force-push protocol.** When a renderer asks the source to
// force-push (e.g. on detected gap), the source replies with a
// snapshot followed by live events. Today `droppedCount > 0` is
// observed but no automatic force-push is issued — the renderer
// continues operating with potentially-stale state until manual
// resync. Closing this is a future PR; for now, the spec contract
// is: *gap detection is logged; recovery is operator-driven via
// `--diag srv` followed by an explicit `Resync` command.*

import { batch } from "solid-js";

/**
 * Wire-format event from any reducer source. Matches
 * `agentmux_common::ipc::Event`'s JSON serialization
 * (`#[serde(tag = "event", rename_all = "snake_case")]`).
 *
 * `event` = discriminant (snake_case), `version` = monotonic
 * per-source counter. Saga lifecycle variants additionally carry
 * `saga_id`. Per-step events MAY carry `saga_id` in a future PR
 * (per-Event threading); today they do not.
 */
export interface VersionedEvent {
    event: string;
    version: number;
    saga_id?: number;
    [field: string]: unknown;
}

/** Subscriber callback — invoked once per event in source order. */
export type EventCallback<E extends VersionedEvent> = (evt: E) => void;

/**
 * Solid signal setters the tracker drives on every dispatch. Optional —
 * a source with no signal-reading consumers (e.g. srv-events.ts, whose
 * `srvEvent`/`srvEventVersion`/`srvEventsActive` signals were removed as
 * dead exports) can omit any/all of them rather than pay for signal
 * writes nobody reads. Kept as plain functions (not a Solid primitive)
 * so the tracker is unit-testable without a Solid runtime.
 */
export interface SignalSetters<E extends VersionedEvent> {
    setLatest?: (evt: E) => void;
    setVersion?: (v: number) => void;
    setSawAny?: (b: boolean) => void;
}

export interface PerSourceTrackerOptions {
    /** Source name for log prefixes — e.g. "srv", "launcher", "host". */
    source: string;
    /**
     * Cap on how many events can pile up in a saga buffer before an
     * emergency flush. Default 1000. The serial-saga server-side
     * timeout is 5s, so 1000 events ≈ 200 events/sec — well past any
     * realistic saga.
     */
    maxSagaBufferSize?: number;
    /**
     * Override the default gap warning. Useful in tests, or for
     * future PRs that want to issue an automatic Resync request.
     */
    onVersionGap?: (gap: number, prevVersion: number, newVersion: number) => void;
    /**
     * Called on every saga terminal (success OR fail), AFTER the
     * buffer has been flushed. Useful for telemetry / tests.
     */
    onSagaTerminal?: (sagaId: number, outcome: "completed" | "failed") => void;
}

export interface PerSourceStats {
    lastVersion: number;
    droppedCount: number;
    sawAnyEvent: boolean;
    /** Saga id of the in-flight saga, or null. */
    inSaga: number | null;
    /** Number of events currently sitting in the saga buffer. */
    bufferedCount: number;
    /** Number of registered subscribers. */
    subscriberCount: number;
}

interface SagaBuffer {
    saga_id: number;
    events: VersionedEvent[];
}

/**
 * Per-source event tracker. One instance per pipe (srv, launcher,
 * host).
 *
 * Wire-up:
 *   ```ts
 *   const [latest, setLatest] = createSignal<SrvEvent | null>(null);
 *   const [version, setVersion] = createSignal(0);
 *   const [sawAny, setSawAny] = createSignal(false);
 *   const tracker = new PerSourceTracker<SrvEvent>(
 *       { source: "srv" },
 *       { setLatest, setVersion, setSawAny },
 *   );
 *   window.__agentmux_srv_event = (evt) => tracker.deliver(evt);
 *   ```
 */
export class PerSourceTracker<E extends VersionedEvent = VersionedEvent> {
    private subscribers = new Set<EventCallback<E>>();
    private lastVersion = 0;
    private droppedCount = 0;
    private sawAnyEvent = false;
    private sagaBuffer: SagaBuffer | null = null;

    private readonly source: string;
    private readonly maxSagaBufferSize: number;
    private readonly onVersionGap: NonNullable<PerSourceTrackerOptions["onVersionGap"]>;
    private readonly onSagaTerminal: NonNullable<PerSourceTrackerOptions["onSagaTerminal"]>;

    constructor(
        opts: PerSourceTrackerOptions,
        private readonly setters: SignalSetters<E>,
    ) {
        this.source = opts.source;
        this.maxSagaBufferSize = opts.maxSagaBufferSize ?? 1000;
        this.onVersionGap =
            opts.onVersionGap ??
            ((gap, prev, next) => {
                console.warn(
                    `[${this.source}-events] version gap: expected ${prev + 1}, got ${next} (${gap} event${gap === 1 ? "" : "s"} possibly dropped)`,
                );
            });
        this.onSagaTerminal = opts.onSagaTerminal ?? (() => {});
    }

    /**
     * Register a per-event callback. Called once per event in
     * source order, including during saga flushes (where signals
     * coalesce into the last-event-only via `solid.batch`, but
     * subscribers see every step).
     *
     * Returns an unsubscribe function.
     */
    subscribe(cb: EventCallback<E>): () => void {
        this.subscribers.add(cb);
        return () => {
            this.subscribers.delete(cb);
        };
    }

    /**
     * Process one event from the wire. The transport layer (host
     * CEF JS bridge) calls this once per event.
     */
    deliver(evt: E): void {
        // Defensive shape check — discard junk so a single malformed
        // event can't tear down the dispatcher.
        if (
            evt == null ||
            typeof evt !== "object" ||
            typeof evt.version !== "number" ||
            typeof evt.event !== "string"
        ) {
            console.warn(`[${this.source}-events] received malformed event`, evt);
            return;
        }

        // Source-restart detection. Both srv and launcher reducers
        // reset `event_version` to 0 on process restart (see
        // `agentmux-srv/src/state.rs::default` and
        // `agentmux-launcher/src/state.rs::default`); after a restart
        // the next emitted event is `version=1`. Without resetting
        // here, every post-restart event would fail the stale gate
        // (`version <= lastVersion`) and be dropped permanently until
        // a full page reload. (codex P1, PR #630.)
        //
        // Heuristic: `version === 1 && lastVersion > 0`. Version=1 is
        // unambiguous — only the source's first event after start
        // bumps the counter from 0 to 1, and we'd only see it now if
        // we'd already processed events from a prior incarnation.
        if (evt.version === 1 && this.lastVersion > 0) {
            console.warn(
                `[${this.source}-events] source restart detected (lastVersion=${this.lastVersion}, new event v=1); resetting tracker state`,
            );
            this.lastVersion = 0;
            this.droppedCount = 0;
            // Discard any in-flight saga buffer — the saga it was
            // tracking is part of the dead source's history; the
            // restart will re-emit fresh state via Snapshot+Resync.
            if (this.sagaBuffer) {
                console.warn(
                    `[${this.source}-events] dropping stale saga buffer for saga ${this.sagaBuffer.saga_id} during restart`,
                );
                this.sagaBuffer = null;
            }
        }

        // Stale: lower-or-equal version than last seen. Drop.
        if (this.lastVersion > 0 && evt.version <= this.lastVersion) {
            console.warn(
                `[${this.source}-events] stale event v=${evt.version} (last=${this.lastVersion}); dropping`,
            );
            return;
        }

        // Gap: version skipped one or more.
        if (this.lastVersion > 0 && evt.version > this.lastVersion + 1) {
            const gap = evt.version - this.lastVersion - 1;
            this.droppedCount += gap;
            this.onVersionGap(gap, this.lastVersion, evt.version);
        }
        this.lastVersion = evt.version;

        if (!this.sawAnyEvent) {
            this.sawAnyEvent = true;
            this.setters.setSawAny?.(true);
        }

        // SagaStarted: open a new saga buffer.
        if (evt.event === "saga_started") {
            const sagaId = typeof evt.saga_id === "number" ? evt.saga_id : null;
            if (sagaId === null) {
                console.warn(`[${this.source}-events] saga_started without saga_id; treating as plain event`);
                this.routeIdleOrBuffered(evt);
                return;
            }
            if (this.sagaBuffer !== null) {
                // Nested-saga not supported today (server-side
                // coordinator runs sagas serially). Flush prior +
                // open new — preserves observability without losing
                // the prior saga's subscribers' state updates.
                console.warn(
                    `[${this.source}-events] nested saga: started ${sagaId} while ${this.sagaBuffer.saga_id} in flight; flushing prior`,
                );
                this.flushSagaBuffer(this.sagaBuffer);
            }
            this.sagaBuffer = { saga_id: sagaId, events: [evt] };
            return;
        }

        // SagaCompleted / SagaFailed: close the matching buffer.
        if (evt.event === "saga_completed" || evt.event === "saga_failed") {
            const sagaId = typeof evt.saga_id === "number" ? evt.saga_id : null;
            if (sagaId !== null && this.sagaBuffer && this.sagaBuffer.saga_id === sagaId) {
                this.sagaBuffer.events.push(evt);
                const buf = this.sagaBuffer;
                this.sagaBuffer = null;
                this.flushSagaBuffer(buf);
                this.onSagaTerminal(
                    sagaId,
                    evt.event === "saga_completed" ? "completed" : "failed",
                );
                return;
            }
            // Terminal without matching start (resync mid-saga, or
            // server-side bug). Two constraints to honor:
            //   1. (reagent P1, PR #630) don't bury the terminal in
            //      an unrelated in-flight saga's buffer — its own
            //      terminal may never come.
            //   2. (codex P1, PR #630 round 2) preserve source
            //      ordering — the mismatched terminal carries a
            //      higher version than the in-flight saga's buffered
            //      events, so dispatching it first would make
            //      `setVersion` go backwards when the buffer flushes.
            // Resolution: flush the in-flight buffer first (treating
            // the buffered saga as terminated-without-completion —
            // that's the failure mode this case represents), THEN
            // dispatch the mismatched terminal. Order preserved,
            // nothing buried.
            if (this.sagaBuffer) {
                console.warn(
                    `[${this.source}-events] terminal for saga ${sagaId} arrived while saga ${this.sagaBuffer.saga_id} was in flight; flushing prior buffer first to preserve ordering`,
                );
                const buf = this.sagaBuffer;
                this.sagaBuffer = null;
                this.flushSagaBuffer(buf);
            }
            this.dispatch(evt);
            return;
        }

        this.routeIdleOrBuffered(evt);
    }

    private routeIdleOrBuffered(evt: E): void {
        if (this.sagaBuffer) {
            this.sagaBuffer.events.push(evt);
            if (this.sagaBuffer.events.length > this.maxSagaBufferSize) {
                console.warn(
                    `[${this.source}-events] saga buffer overflow at ${this.sagaBuffer.events.length}; emergency flush of saga ${this.sagaBuffer.saga_id}`,
                );
                const buf = this.sagaBuffer;
                this.sagaBuffer = null;
                this.flushSagaBuffer(buf);
            }
            return;
        }
        this.dispatch(evt);
    }

    private flushSagaBuffer(buf: SagaBuffer): void {
        // `batch` coalesces signal updates: SolidJS effects on
        // `latestEvent` see only the LAST event in this burst. Per-
        // event subscriber callbacks still fire individually.
        batch(() => {
            for (const e of buf.events) {
                this.dispatch(e as E);
            }
        });
    }

    private dispatch(evt: E): void {
        this.setters.setLatest?.(evt);
        this.setters.setVersion?.(evt.version);
        for (const cb of this.subscribers) {
            try {
                cb(evt);
            } catch (err) {
                console.error(`[${this.source}-events] subscriber threw:`, err);
            }
        }
    }

    /** Diagnostic snapshot. Used by `--diag srv` / tests. */
    stats(): PerSourceStats {
        return {
            lastVersion: this.lastVersion,
            droppedCount: this.droppedCount,
            sawAnyEvent: this.sawAnyEvent,
            inSaga: this.sagaBuffer ? this.sagaBuffer.saga_id : null,
            bufferedCount: this.sagaBuffer ? this.sagaBuffer.events.length : 0,
            subscriberCount: this.subscribers.size,
        };
    }
}
