// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Close one tab of an agent pane. Same as the generic `closeBlockInStack`
 * except for one case: closing the LAST tab of this pane while it holds a
 * loaded agent returns the pane to My Agents instead of closing the pane.
 *
 * It does that by adding a fresh, blank agent tab first (which opens on the
 * picker, exactly like "+" → Agent) and only then closing the old tab —
 * which by then is no longer the last member, so it goes through the
 * ordinary delete path (`DeleteBlock` → controller kill, shell sub-block
 * cleanup). Clearing the old block's agent meta in place
 * (`AgentViewModel.backToPicker`) was rejected: it leaves the CLI running
 * and stale meta behind.
 *
 * Every other case is unchanged: a non-last tab closes and activates its
 * neighbor, a picker tab (no agentId) that is the last tab closes the pane,
 * and a fork pill that lives in ANOTHER pane closes that fork the ordinary
 * way (closing that pane if it was its only tab) — the picker fallback only
 * applies to the pane the × was clicked in.
 *
 * The busy-agent close confirmation (`beforeNodeDelete`) is asked BEFORE the
 * picker tab is added, and the close then skips asking again. Asking after
 * would leave a stray My Agents tab behind whenever the user cancels.
 *
 * `layoutModel` is the pane's own (`nodeModel.layoutModel`), not the
 * globally-active tab's — SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md.
 *
 * Spec: SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §2.
 */

import { atoms, MOS } from "@/app/store/global";
import { holdLeafRevealGate, scheduleLeafRevealLift } from "@/app/store/tab-reveal";
import { addWidgetAsPaneTab, closeBlockInStack } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";

const AGENT_WIDGET_KEY = "defwidget@agent";

// In-flight closes keyed by blockId. The picker path awaits a `pane.open`
// RPC before closing, so a fast double-click on × would otherwise add two
// picker tabs. A repeat call while one is in flight awaits the same promise.
const inFlightCloses = new Map<string, Promise<void>>();

type CloseAgentTabOpts = { layoutModel: LayoutModel; ownNodeId: string; blockId: string };

export function closeAgentTab(opts: CloseAgentTabOpts): Promise<void> {
    const inFlight = inFlightCloses.get(opts.blockId);
    if (inFlight) return inFlight;
    const promise = closeAgentTabImpl(opts).finally(() => inFlightCloses.delete(opts.blockId));
    inFlightCloses.set(opts.blockId, promise);
    return promise;
}

async function closeAgentTabImpl({ layoutModel, ownNodeId, blockId }: CloseAgentTabOpts): Promise<void> {
    const node = layoutModel.getNodeByBlockId(blockId);
    if (!node) return;

    if (!shouldReturnToPicker(node, ownNodeId, blockId)) {
        await closeBlockInStack(layoutModel, node.id, blockId);
        return;
    }

    if (layoutModel.beforeNodeDelete && !(await layoutModel.beforeNodeDelete({ blockId } as TabLayoutData))) return;

    // Hide the leaf while the stack swaps members — same flicker guard as
    // openOrFocusHistoryTab (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md).
    const revealGen = holdLeafRevealGate(node.id);
    try {
        try {
            await addWidgetAsPaneTab(layoutModel, node.id, pickerBlockDef());
        } catch (e) {
            // Couldn't create the replacement — fall back to today's
            // behavior rather than leaving the tab un-closable.
            console.error("closeAgentTab: failed to open a My Agents tab, closing the pane instead", e);
        }
        // Re-resolve: the pane could have closed while pane.open was in
        // flight. If it has, the old block went with it.
        const freshNode = layoutModel.getNodeByBlockId(blockId);
        if (!freshNode) return;
        await closeBlockInStack(layoutModel, freshNode.id, blockId, { confirmed: true });
    } finally {
        scheduleLeafRevealLift(node.id, revealGen);
    }
}

function shouldReturnToPicker(node: { id: string; data?: TabLayoutData }, ownNodeId: string, blockId: string): boolean {
    if (node.id !== ownNodeId) return false; // a fork pill owned by another pane
    const stack = node.data?.blockStack?.length ? node.data.blockStack : [node.data?.blockId];
    if (stack.length > 1) return false;
    const meta = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta;
    return meta?.view === "agent" && !!meta?.["agentId"];
}

/** The same block definition "+" → Agent uses, so the replacement tab is
 *  indistinguishable from a freshly added one. */
function pickerBlockDef(): BlockDef {
    return atoms.fullConfigAtom()?.widgets?.[AGENT_WIDGET_KEY]?.blockdef ?? { meta: { view: "agent" } };
}
