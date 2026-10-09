// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { isNewerLedger, parseTurnLedger, type TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import * as MOS from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { AmbientNarrationNode } from "../types";

/**
 * The latest ledger seen per block, outliving the pane. A remount on the same
 * connection gets no replay of the persisted event (the broker replays once
 * per route), while the pane's own state slot starts empty again, so without
 * this the clock and counter would restart on every tab switch (#4492). One
 * small object per block ever opened this session.
 */
const latestLedger = new Map<string, TurnLedger>();

/** The latest ledger seen for `blockId` this session (tests). */
export function cachedTurnLedger(blockId: string): TurnLedger | undefined {
    return latestLedger.get(blockId);
}

/** A wake-up line is added only for a turn this recent: an old turn coming
 *  back (a cache re-feed, a replay) is not news. */
const WAKE_LINE_FRESH_MS = 15_000;

/**
 * The transcript line for a turn the CLI started by itself for a finished
 * background task, which leaves no message of its own in the transcript.
 * Rendered as an ambient line (marked as AgentMux's, not the model's), keyed
 * by the turn so a remount or replay never adds it twice.
 */
export function taskWakeLine(ledger: TurnLedger, nowMs: number): AmbientNarrationNode | null {
    if (ledger.trigger?.kind !== "task" || ledger.passes !== 1 || !ledger.active) return null;
    if (nowMs - ledger.startedAtMs > WAKE_LINE_FRESH_MS) return null;
    return {
        type: "ambient_narration",
        id: `turn-trigger-${ledger.turnId}`,
        kind: "turn_trigger",
        text: `Woke up: ${ledger.trigger.from ?? "a background task finished"}`,
        timestamp: ledger.startedAtMs,
    };
}

/**
 * Feed srv's turn ledger (`agentturn`) into the pane reducer as `TurnObserved`
 * (the cached one at once on mount, then every change), and mark a task
 * wake-up in the transcript.
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4, §5.3.
 */
export function useTurnLedger(blockId: string, paneModel: Pick<AgentPaneModel, "dispatchPane" | "dispatchDoc">): void {
    onMount(() => {
        const cached = latestLedger.get(blockId);
        if (cached) paneModel.dispatchPane({ type: "TurnObserved", ledger: cached }, "system");
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.AgentTurn,
            scope: MOS.makeORef("block", blockId),
            handler: (event) => {
                const ledger = parseTurnLedger((event as { data?: unknown })?.data);
                if (!ledger) return;
                if (isNewerLedger(ledger, latestLedger.get(blockId))) latestLedger.set(blockId, ledger);
                paneModel.dispatchPane({ type: "TurnObserved", ledger }, "system");
                const line = taskWakeLine(ledger, Date.now());
                if (line) paneModel.dispatchDoc({ type: "StreamFlush", newNodes: [line], updatedNodes: [] });
            },
        });
        onCleanup(() => unsub?.());
    });
}
