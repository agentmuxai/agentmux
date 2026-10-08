// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { parseTurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import * as MOS from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

/**
 * Feed srv's turn ledger (`agentturn`, persisted, so a pane that mounts
 * mid-turn gets the latest at once) into the pane reducer as `TurnObserved`.
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4.
 */
export function useTurnLedger(blockId: string, paneModel: Pick<AgentPaneModel, "dispatchPane">): void {
    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.AgentTurn,
            scope: MOS.makeORef("block", blockId),
            handler: (event) => {
                const ledger = parseTurnLedger((event as { data?: unknown })?.data);
                if (ledger) paneModel.dispatchPane({ type: "TurnObserved", ledger }, "system");
            },
        });
        onCleanup(() => unsub?.());
    });
}
