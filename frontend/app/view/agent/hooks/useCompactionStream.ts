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
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
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

export function useCompactionStream(opts: UseCompactionStreamOptions): void {
    const unsub = waveEventSubscribe({
        eventType: WpsEvent.CompactionStarted,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const data = event?.data;
            if (!data || typeof data !== "object") return;
            const trigger =
                data.trigger === "auto" ? "auto" :
                data.trigger === "manual" ? "manual" :
                null;
            if (!trigger) return;

            const at = Date.now();
            opts.model.dispatchPane({ type: "CompactionStarted", trigger, at });

            const node: CompactionStartedNode = {
                type: "compaction_started",
                id: `compaction-started-${at}`,
                trigger,
                startedAt: at,
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
