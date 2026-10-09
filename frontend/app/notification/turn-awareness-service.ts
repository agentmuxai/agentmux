// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * App-wide feed for `store/turn-awareness.ts`: every block's turn ledger, and
 * whether the user is looking at that block (this window focused, the block
 * focused in it). Installed once per window with the other window services.
 */

import { focusManager } from "@/app/store/focusManager";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { installTurnAwareness } from "@/app/store/turn-awareness";
import { makeWindowFocusSignal } from "@/app/window/window-focus";

export function installTurnAwarenessService(): () => void {
    const windowFocused = makeWindowFocusSignal();
    return installTurnAwareness({
        subscribe: (handler) =>
            muxEventSubscribe({
                eventType: WpsEvent.AgentTurn,
                // No scope: every block's.
                handler: (event) => {
                    const data = (event as { data?: { block_id?: unknown } })?.data;
                    if (typeof data?.block_id === "string") handler(data.block_id, data);
                },
            }),
        watching: (blockId) => windowFocused() && focusManager.blockFocusAtom() === blockId,
    });
}
