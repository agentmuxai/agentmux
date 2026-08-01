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
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { AgentPaneEvent } from "@/app/store/agent-pane-state/types";
import type { CompactionStartedNode } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseCompactionStreamOptions {
    blockId: string;
    model: AgentPaneModel;
    queue: StreamFlushQueue;
    /** In-batch dedup cache shared with the rest of useAgentStream's producers. */
    hasNodeId: (id: string) => boolean;
    addNodeId: (id: string) => void;
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
 * Whether the reducer actually accepted a dispatched `CompactionStarted`
 * command. The reducer's round-5 fix (`workingFromPhase` gate) makes it a
 * no-op — empty `events`, state unchanged — when this ping arrives after
 * the turn's own `TurnEnd` already fired on the separate NDJSON stream, a
 * real and expected race given `compaction_started`'s separate WPS
 * transport. Pure and exported for direct unit coverage (reagent P1, PR
 * #2378 round 6): without this check, a stray ping the reducer correctly
 * rejected would still get a permanent "Compacting conversation…"
 * transcript node pushed for a compaction that isn't actually happening.
 */
export function wasCompactionStartedAccepted(paneEvents: AgentPaneEvent[]): boolean {
    return paneEvents.some((ev) => ev.type === "compaction-started");
}

export function useCompactionStream(opts: UseCompactionStreamOptions): void {
    const unsub = waveEventSubscribe({
        eventType: WpsEvent.CompactionStarted,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const resolved = resolveCompactionStart(event?.data, Date.now());
            if (!resolved) return;
            const { trigger, startedAt } = resolved;

            const paneEvents = opts.model.dispatchPane({ type: "CompactionStarted", trigger, at: startedAt });
            if (!wasCompactionStartedAccepted(paneEvents)) return;

            const node: CompactionStartedNode = {
                type: "compaction_started",
                id: `compaction-started-${startedAt}`,
                trigger,
                startedAt,
            };
            if (!opts.hasNodeId(node.id)) {
                opts.addNodeId(node.id);
                opts.queue.pushNewNode(node);
                opts.queue.scheduleFlush();
            }
        },
    });

    // Own the subscription at body scope so it is torn down even if the
    // caller's onMount early-returns (e.g. enabled:false). Without this
    // the global handler would leak one per mount.
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
