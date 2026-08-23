// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open (or focus, if already open) the Agent History tab for a given agent,
 * as a sibling tab in the SAME pane as `currentBlockId`. Replaces the old
 * `bodyMode: "history"` in-place content swap — history is now a normal
 * blockStack tab (same mechanism forks and the "+" new-tab button already
 * use), never a toggle inside one tab's own component tree.
 *
 * Shared by every entry point so "already open → focus, don't duplicate"
 * lives in exactly one place: the scrolling `history_link` row
 * (DocumentRow.tsx), AgentControlBar's menu entry, and the right-click
 * context menu (AgentViewModel.getBodyContextMenuItems).
 *
 * Standalone (not a component/hook) — `getLayoutModelForStaticTab()` reads
 * a plain global atom, not SolidJS component context, so this is safely
 * callable from anywhere, including a ViewModel method that has no
 * reactive-owner scope of its own.
 *
 * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1,
 * SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md (the leaf-scoped reveal
 * gate wrapping both the create-new and switch-to-existing paths below).
 */

import { getLayoutModelForStaticTab, pushBlockOntoStack, setActiveBlockInStack } from "@/layout/index";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ObjectService } from "@/app/store/services";
import { pushNotification, WOS } from "@/app/store/global";
import { holdLeafRevealGate, scheduleLeafRevealLift } from "@/app/store/tab-reveal";

/** Block-meta key marking a block as a read-only history reader for the
 *  named agent, rather than a live launch. Never set alongside a live
 *  conversation's own launch meta. */
export const HISTORY_TAB_FOR_META_KEY = "agent:historyTabFor";

/** Block-meta key carrying the ORIGINAL live block's id — the history
 *  tab's own block is never actually launched, so it has no local output
 *  of its own; `AgentHistoryView` reads transcripts through this id
 *  instead of its own (`sourceBlockId` — see that component's doc
 *  comment), so the backend's own-block fallback path (when the global
 *  transcript zone lookup itself comes up empty) can't silently land on
 *  an empty block. codex P1 on PR #2539. */
export const HISTORY_SOURCE_BLOCK_ID_META_KEY = "agent:historySourceBlockId";

/** True if `blockId` is (or has ever been opened as) a history-tab block
 *  for `agentId`. Reads persisted meta directly (WOS store), not a
 *  reactive block atom — this runs outside any component's reactive scope
 *  and only needs a point-in-time answer. */
function isHistoryTabFor(blockId: string, agentId: string): boolean {
    const meta = WOS.getObjectValue<Block>(WOS.makeORef("block", blockId))?.meta;
    return meta?.[HISTORY_TAB_FOR_META_KEY] === agentId;
}

// In-flight opens, keyed by `${currentBlockId}|${agentId}`. Without this,
// two near-simultaneous calls (a double-click, or the link row and the
// context menu entry both firing before the first `pane.open` RPC
// resolves) each read the same pre-RPC blockStack, both miss the
// not-yet-created tab, and both push a duplicate — violating the
// open-OR-focus guarantee. codex P2 / reagent P2 on PR #2539. A second
// call while the first is still in flight AWAITS THE SAME PROMISE instead
// of re-running the body, so it does no work of its own and can't race.
const inFlightOpens = new Map<string, Promise<void>>();

export async function openOrFocusHistoryTab(opts: { currentBlockId: string; agentId: string }): Promise<void> {
    const key = `${opts.currentBlockId}|${opts.agentId}`;
    const inFlight = inFlightOpens.get(key);
    if (inFlight) return inFlight;

    const promise = openOrFocusHistoryTabImpl(opts);
    inFlightOpens.set(key, promise);
    try {
        await promise;
    } finally {
        inFlightOpens.delete(key);
    }
}

async function openOrFocusHistoryTabImpl(opts: { currentBlockId: string; agentId: string }): Promise<void> {
    const { currentBlockId, agentId } = opts;
    const layoutModel = getLayoutModelForStaticTab();
    const node = layoutModel.getNodeByBlockId(currentBlockId);
    if (!node) return;

    // Hide this pane while it settles — both branches below
    // (setActiveBlockInStack for an already-open history tab, or
    // pane.open + pushBlockOntoStack for a fresh one) force the same
    // remount `layoutStack.ts`'s own doc comment describes.
    // SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md.
    holdLeafRevealGate(node.id);
    try {
        const stack = node.data?.blockStack?.length ? node.data.blockStack : [currentBlockId];
        const existing = stack.find((id) => isHistoryTabFor(id, agentId));
        if (existing) {
            setActiveBlockInStack(layoutModel, node.id, existing);
            return;
        }

        // Copy the display fields AgentHistoryTabView reads off ITS OWN block
        // meta (mirrors how the live pane reads its own `agentOutputFormat`/
        // `agentName` — same read shape, different block) — a bare
        // `agent:historyTabFor` block otherwise has no provider/name info of
        // its own, since it's never actually launched.
        const liveMeta = WOS.getObjectValue<Block>(WOS.makeORef("block", currentBlockId))?.meta;

        let paneOpenResult: { block_id: string };
        try {
            paneOpenResult = (await TabRpcClient.rpcCall(
                "pane.open",
                {
                    view: "agent",
                    skip_placement: true,
                    meta: {
                        view: "agent",
                        agentId,
                        [HISTORY_TAB_FOR_META_KEY]: agentId,
                        [HISTORY_SOURCE_BLOCK_ID_META_KEY]: currentBlockId,
                        agentOutputFormat: liveMeta?.["agentOutputFormat"],
                        agentName: liveMeta?.["agentName"],
                    },
                },
                {},
            )) as { block_id: string };
        } catch (e: unknown) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Agent History failed to open",
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
            return;
        }

        // The pane could have closed while the RPC above was in flight —
        // re-resolve fresh rather than trusting the pre-await `node`
        // reference (same defensive check as AgentViewWrapper's handleNewAgentTab).
        const freshNode = layoutModel.getNodeByBlockId(currentBlockId);
        if (!freshNode) {
            await ObjectService.DeleteBlock(paneOpenResult.block_id).catch(() => {});
            return;
        }
        pushBlockOntoStack(layoutModel, freshNode.id, paneOpenResult.block_id);
    } finally {
        // Pair with holdLeafRevealGate above — runs on every exit path.
        scheduleLeafRevealLift(node.id);
    }
}
