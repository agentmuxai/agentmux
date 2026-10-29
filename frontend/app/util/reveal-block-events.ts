// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Receiving side of `block.reveal` (SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md
 * §4.3). A renderer can't drive another window's layout, so when `revealBlock`
 * finds the block isn't here it asks srv, which publishes `block:reveal`
 * naming the window that shows the block. Only that window acts, revealing
 * the block itself — window tab, pane tab, pane focus, caret.
 */

import { onMuxEvent } from "@/app/store/mps";
import { windowId } from "@/app/store/window-identity";
import { isFailure, type SelectionResult } from "@/app/selection/selection-machine";
import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { revealBlock } from "@/app/util/reveal-block";
import type { BlockRevealEvent } from "@/types/rpc/BlockRevealEvent";

import { WpsEvent } from "@/app/store/mps-events";
export const EVENT_BLOCK_REVEAL = WpsEvent.BlockReveal;

/** Is this `block:reveal` for THIS window? srv always names one window. */
export function shouldRevealHere(d: Partial<BlockRevealEvent> | undefined, myWindowId: string): boolean {
    return !!d?.block_id && !!myWindowId && d.window_id === myWindowId;
}

/** Bring this window to the front; best-effort. */
async function raiseThisWindow(): Promise<void> {
    try {
        const label = await getApi().getWindowLabel();
        if (label) await getApi().focusWindow(label);
    } catch {
        /* the sender raises it too, once it has heard back */
    }
}

/** Tell srv how the request went, for the window that waits on it. */
async function answer(requestId: string, result: SelectionResult): Promise<void> {
    try {
        await RpcApi.BlockRevealedCommand(TabRpcClient, {
            request_id: requestId,
            ok: !isFailure(result),
            reason: isFailure(result) ? result.reason : undefined,
        });
    } catch (e) {
        console.warn("[reveal-block] block.revealed failed", String(e));
    }
}

/** Handle one `block:reveal` payload. Returns whether this window revealed it. */
export async function handleBlockReveal(d: Partial<BlockRevealEvent> | undefined, myWindowId: string): Promise<boolean> {
    if (!shouldRevealHere(d, myWindowId)) return false;
    const asked = !!d!.request_id;
    // A hidden or minimized window draws no frames, and the read-back waits for two: raise it before showing
    // the pane, not after the sender has heard back (which it can't until this window answers).
    if (asked) await raiseThisWindow();
    // A caller that waits tells the user itself when this fails; one that does not (an agent's pane open) leaves it to us.
    const result = await revealBlock(d!.block_id!, { tabId: d!.tab_id, source: "remote-window", localOnly: true, quiet: asked });
    const revealed = !isFailure(result);
    if (isFailure(result)) {
        const why = result.reason === "not-found" || result.reason === "gone" ? "block not in this window" : `could not show it (${result.reason})`;
        console.warn(`[reveal-block] block:reveal: ${why}`, d);
    }
    if (asked) await answer(d!.request_id!, result);
    return revealed;
}

let installed = false;

/** Subscribe this window to `block:reveal`. Once per window. */
export function installBlockRevealEvents(): () => void {
    if (installed) return () => {};
    installed = true;
    const unsub = onMuxEvent(EVENT_BLOCK_REVEAL, (reveal) => {
        void handleBlockReveal(reveal, windowId()).catch((e) => console.warn("[reveal-block] block:reveal failed", String(e)));
    });
    return () => {
        unsub();
        installed = false;
    };
}
