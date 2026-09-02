// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useCompactionStream — the single per-block WPS subscription for
 * `compaction_started` events, published by the `PreCompact` hook
 * (`agentmux-bashwrap precompact --trigger=manual|auto`) the instant
 * Claude Code begins compacting. See
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2 /
 * Tier 1.
 *
 * Mirrors `useToolChunkStream.ts`'s contract exactly: a single per-block
 * subscription, pushing into the shared `StreamFlushQueue` rather than
 * dispatching its own flush/batch — a second independent RAF/`batch()`
 * here would reintroduce the reconcileArrays/replaceChild crash
 * documented in RETRO_REPLACECHILD_CRASH_2026-06-06.md.
 *
 * Installed at BODY scope by the caller (called directly, not from
 * inside `onMount`) — same early-return-safety rationale as
 * `useToolChunkStream`.
 *
 * This hook only knows about the START signal. The matching completion
 * data (real `trigger`/`preTokens`/`postTokens`/`durationMs`) arrives
 * later as a `compact_boundary` frame on the normal NDJSON stream —
 * handled directly in `useAgentStream.ts`, which also clears the
 * `compacting` pane flag this hook sets.
 *
 * Codex P1 on PR #2378 (two rounds): this event is published with
 * `persist: 0` (`wps_client.rs`) — never retained/replayed — because
 * there is no completion tombstone (`compact_boundary` arrives over
 * the separate NDJSON stream, not WPS), so a replayed "started" ping
 * is indistinguishable from a genuinely active one and a timestamp-
 * age guard alone cannot fix that on the receiving end: a pane
 * reconnecting seconds after a real, already-finished compaction
 * would still fall well within any plausible-duration window. Only a
 * currently-live subscriber ever sees this event now. The `startedAt`
 * staleness check below is kept as defense-in-depth against a
 * malformed/delayed live delivery, not as the mechanism preventing
 * stale replays — that's `persist: 0`'s job.
 *
 * SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md: a ping can
 * ALSO arrive too EARLY — before a freshly-mounted pane's `turnPhase` has
 * been reconciled out of the mount-default `Idle` (`ReconcileTurnActive`
 * is an async RPC round-trip). That case is buffered in the reducer
 * itself (`state.pendingCompactionPing`) and retroactively promoted into
 * `state.compacting` by `ReconcileTurnActive`/`StreamFlushObserved` — see
 * those cases in `reducer.ts`. Two earlier fix attempts tried to handle
 * this entirely inside THIS hook (a local retry keyed off observing
 * `turnPhase`, then a time-bounded version) and were both structurally
 * unsound (PR #2928 review, reagent P1 + codex P2, multiple rounds): a
 * hook observing only the RESULTING value of a signal cannot reliably
 * tell "this promotion is the same turn a buffered ping was about" from
 * "this is a later, unrelated turn," the way the reducer can by reacting
 * to the discrete command itself.
 *
 * That same lesson applies to pushing the "Compacting conversation…"
 * transcript node: a promoted `pendingCompactionPing` is applied from
 * `ReconcileTurnActive` (dispatched in `agent-view.tsx`) or
 * `StreamFlushObserved` (dispatched in `stream-flush-queue.ts`) — neither
 * call site has access to this hook's document-node queue or dedup
 * cache, and inspecting their individual dispatch return values from here
 * would mean duplicating this hook's own push logic at each of those
 * sites (reagent P1, PR #2928, third round). Instead, this hook watches
 * the reactive `compacting` signal itself (kept in sync with
 * `state.compacting` by `registerAgentPane`'s generic projection,
 * regardless of which dispatch changed it) and pushes the node whenever
 * it transitions from `null` to set — one place, correct for every
 * promotion path, present and future, without needing to know which
 * dispatch caused it.
 */

import { createEffect, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { CompactionState } from "@/app/store/agent-pane-state/types";
import type { CompactionStartedNode } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseCompactionStreamOptions {
    blockId: string;
    model: AgentPaneModel;
    queue: StreamFlushQueue;
    /** In-batch dedup cache shared with the rest of useAgentStream's producers. */
    hasNodeId: (id: string) => boolean;
    addNodeId: (id: string) => void;
    /**
     * Reactive `state.compacting` for this pane — pushes the transcript
     * node whenever it transitions from `null` to set. See the module doc
     * comment for why this replaces inspecting individual dispatch return
     * values. Threaded down from `useAgentStream.ts`'s own `compactingAtom`.
     */
    compacting: Accessor<CompactionState | null>;
}

/**
 * A `compaction_started` ping older than this is treated as stale —
 * almost certainly a WPS replay of a compaction that already
 * finished (its `compact_boundary` completion arrived over the
 * separate NDJSON stream while nobody was subscribed to see it), not
 * a real one still in flight. The real captured example in the spec
 * doc (§2) took ~232s; 10 minutes is a generous multiple of that so
 * a genuinely slow compaction is never misclassified as stale.
 */
const MAX_PLAUSIBLE_COMPACTION_MS = 10 * 60 * 1000;

/**
 * Tolerance for `startedAt` appearing slightly in the future relative
 * to this client's clock (delivery jitter / minor clock skew between
 * this machine and wherever the hook process's clock reads from —
 * both are local to the same host today, but this stays defensive).
 * A genuinely stale replay is stale by minutes, not milliseconds, so
 * this tolerance can't mask the bug the age check exists to catch.
 */
const CLOCK_SKEW_TOLERANCE_MS = 60 * 1000;

export type CompactionTrigger = "manual" | "auto";

/**
 * Validate + resolve a raw `compaction_started` WPS payload into a
 * trigger + clamped `startedAt`, or `null` to reject it outright
 * (malformed shape, unparseable/missing timestamp, or stale replay —
 * see the module doc comment). Pure and exported so the staleness
 * logic has direct unit coverage without spinning up a SolidJS
 * reactive-root harness for the subscription wiring itself.
 */
export function resolveCompactionStart(
    data: unknown,
    now: number,
): { trigger: CompactionTrigger; startedAt: number } | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    const trigger: CompactionTrigger | null =
        d.trigger === "auto" ? "auto" :
        d.trigger === "manual" ? "manual" :
        null;
    if (!trigger) return null;

    // Malformed/missing startedAt degrades to "reject" (fail closed),
    // not "treat as fresh" — same "skip rather than guess" philosophy
    // as the backend translator. In practice this binary always sends
    // a valid RFC3339 timestamp (precompact.rs), so a parse failure
    // here would itself indicate something is wrong worth not acting on.
    const rawStartedAt = typeof d.startedAt === "string" ? Date.parse(d.startedAt) : NaN;
    if (Number.isNaN(rawStartedAt)) return null;
    const age = now - rawStartedAt;
    if (age < -CLOCK_SKEW_TOLERANCE_MS || age > MAX_PLAUSIBLE_COMPACTION_MS) return null;
    // Clamp a within-tolerance "future" timestamp to now, so the live
    // elapsed-time display (Date.now() - startedAt) never briefly
    // reads negative.
    return { trigger, startedAt: Math.min(rawStartedAt, now) };
}

/**
 * Build the transcript node id for a given compaction start time. Exported
 * so other producers of a `compacting` transition (there are none today
 * outside the reducer cases this hook's effect already covers) would key
 * dedup identically if one were ever added.
 */
export function compactionStartedNodeId(startedAt: number): string {
    return `compaction-started-${startedAt}`;
}

export function useCompactionStream(opts: UseCompactionStreamOptions): void {
    const unsub = waveEventSubscribe({
        eventType: WpsEvent.CompactionStarted,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const resolved = resolveCompactionStart(event?.data, Date.now());
            if (!resolved) return;
            // Dispatch only — the reducer decides accept / buffer / reject.
            // The `compacting` effect below pushes the transcript node
            // uniformly, whether this ping is accepted immediately or (if
            // buffered) promoted later by a different dispatch entirely.
            opts.model.dispatchPane({ type: "CompactionStarted", trigger: resolved.trigger, at: resolved.startedAt });
        },
    });

    // Pushes the "Compacting conversation…" transcript node exactly once
    // per real compaction, the instant `state.compacting` is confirmed set
    // — regardless of whether that happened via this hook's own live
    // dispatch above, or a `pendingCompactionPing` promoted later by
    // `ReconcileTurnActive` or `StreamFlushObserved` (dispatched from
    // agent-view.tsx / stream-flush-queue.ts respectively — see the module
    // doc comment for why observing the signal, not those call sites'
    // return values, is what makes this correct for every promotion path).
    createEffect(() => {
        const compacting = opts.compacting();
        if (!compacting) return;
        const node: CompactionStartedNode = {
            type: "compaction_started",
            id: compactionStartedNodeId(compacting.startedAt),
            trigger: compacting.trigger,
            startedAt: compacting.startedAt,
        };
        if (!opts.hasNodeId(node.id)) {
            opts.addNodeId(node.id);
            opts.queue.pushNewNode(node);
            opts.queue.scheduleFlush();
        }
    });

    // Own the subscription at body scope so it is torn down even if the
    // caller's onMount early-returns (e.g. enabled:false). Without this
    // the global handler would leak one per mount.
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
