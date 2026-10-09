// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Receiving side of `block.reveal` (SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md
 * §4.3). A renderer can't drive another window's layout, so when `revealBlock`
 * finds the block isn't here it asks srv, which publishes `block:reveal`
 * naming the window that shows the block. Only that window acts, revealing
 * the block itself — window tab, pane tab, pane focus, caret.
 */

import { muxEventSubscribe } from "@/app/store/mps";
import { windowId } from "@/app/store/window-identity";
import { revealBlockLocally } from "@/app/util/reveal-block";
import type { BlockRevealEvent } from "@/types/rpc/BlockRevealEvent";

import { WpsEvent } from "@/app/store/mps-events";
export const EVENT_BLOCK_REVEAL = WpsEvent.BlockReveal;

/** Is this `block:reveal` for THIS window? srv always names one window. */
export function shouldRevealHere(d: Partial<BlockRevealEvent> | undefined, myWindowId: string): boolean {
    return !!d?.block_id && !!myWindowId && d.window_id === myWindowId;
}

/** Handle one `block:reveal` payload. Returns whether this window revealed it. */
export async function handleBlockReveal(d: Partial<BlockRevealEvent> | undefined, myWindowId: string): Promise<boolean> {
    if (!shouldRevealHere(d, myWindowId)) return false;
    const revealed = await revealBlockLocally(d!.block_id!, { tabId: d!.tab_id });
    if (!revealed) console.warn("[reveal-block] block:reveal: block not in this window", d);
    return revealed;
}

let installed = false;

/** Subscribe this window to `block:reveal`. Once per window. */
export function installBlockRevealEvents(): () => void {
    if (installed) return () => {};
    installed = true;
    const unsub = muxEventSubscribe({
        eventType: EVENT_BLOCK_REVEAL,
        handler: (event) => {
            void handleBlockReveal(event.data as BlockRevealEvent | undefined, windowId()).catch((e) =>
                console.warn("[reveal-block] block:reveal failed", String(e))
            );
        },
    });
    return () => {
        unsub();
        installed = false;
    };
}
