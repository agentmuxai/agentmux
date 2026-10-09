// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { parseTurnLedger, type TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import * as MOS from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

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

/**
 * Feed srv's turn ledger (`agentturn`) into the pane reducer as `TurnObserved`:
 * the cached one at once on mount, then every change.
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4.
 */
export function useTurnLedger(blockId: string, paneModel: Pick<AgentPaneModel, "dispatchPane">): void {
    onMount(() => {
        const cached = latestLedger.get(blockId);
        if (cached) paneModel.dispatchPane({ type: "TurnObserved", ledger: cached }, "system");
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.AgentTurn,
            scope: MOS.makeORef("block", blockId),
            handler: (event) => {
                const ledger = parseTurnLedger((event as { data?: unknown })?.data);
                if (!ledger) return;
                const prev = latestLedger.get(blockId);
                if (!prev || ledger.turnId >= prev.turnId) latestLedger.set(blockId, ledger);
                paneModel.dispatchPane({ type: "TurnObserved", ledger }, "system");
            },
        });
        onCleanup(() => unsub?.());
    });
}
