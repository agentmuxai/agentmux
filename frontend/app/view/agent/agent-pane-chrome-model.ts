// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 1).

import { setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import { closeAgentTab } from "./close-agent-tab";

/**
 * Chrome half of the agent pane — the header, tab strip and progress-bar
 * slot, rendered by the shared `renderPaneChromeShell` (pane-leaf-chrome.tsx's
 * fallback when a view supplies no `renderPaneChrome`) for EVERY agent pane
 * (see `pane-leaf-chrome.tsx`'s `hoisted` memo for why gating this on stack
 * size was a catch-22), wrapping whichever `AgentBlockContent` instance is
 * currently the active stack member's own switch-scoped `<Block>`
 * (`content` prop, supplied by `pane-leaf-chrome.tsx`). Unlike `AgentBlockContent`, this component is
 * constructed ONCE per leaf and stays mounted across every subsequent
 * switch — that persistence is the entire point of this file's split
 * (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`).
 *
 * `anchorBlockId` is the blockId of whichever stack member's `ViewModel`
 * FIRST called `renderPaneChrome` — frozen for this component's entire
 * lifetime, the same "one instance, one immutable blockId" contract every
 * other `ViewModel`/`NodeModel` consumer already follows. It is NOT used to
 * resolve the owning `LayoutNode`, though — ReAgent P0 on #3136:
 * `anchorBlockId` can itself be closed by the user (removed from
 * `blockStack` by `closeBlockInStack`'s `filter`), and chrome never unmounts
 * to recover — a `getNodeByBlockId(anchorBlockId)` lookup that outlives that
 * tab's closure would return `null` forever after, breaking the whole tab
 * strip. Node resolution uses `nodeModel.nodeId` instead
 * (`getOwnNode`, below) — the leaf's own id, stable regardless of which
 * stack members come and go.
 */
/**
 * Agent's opt-in to the ONE shared pane chrome
 * (`renderPaneChromeShell`) — see `PaneChromeModel` (custom.d.ts).
 * Everything the old `AgentPaneChrome` component rendered around the
 * content (root box, focus ring, header row, ErrorBoundary) is the shared
 * chrome's job now; what stays here is only what is genuinely agent's:
 * its tab model (this pane's own stack, rename,
 * per-pane zoom) and the below-header slot its turn-progress bar portals
 * into. That slot is a generic capability — any view type can take it.
 */
export function buildAgentPaneChromeModel(anchorBlockId: string, nodeModel: NodeModel): PaneChromeModel {
    // `nodeModel.layoutModel`, NOT `getLayoutModelForStaticTab()` — see that
    // field's own doc comment (layout/lib/types.ts) and
    // SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md: this pane's
    // own tab is not necessarily "whichever tab is globally active right
    // now" at the moment chrome first constructs (e.g. a brand-new tab's
    // default agent pane, seeded by `applyTabPreset` before `setActiveTab`
    // ever runs).
    const layoutModel = nodeModel.layoutModel;

    // ReAgent P0 on this PR: resolving the owning node via
    // layoutModel.getNodeByBlockId(anchorBlockId) — anchorBlockId's own
    // originating tab — breaks permanently the moment the user closes THAT
    // specific tab: closeBlockInStack removes a closed member from
    // blockStack via filter, so getNodeByBlockId(anchorBlockId) would
    // return null forever after (chrome never unmounts to recover once
    // mounted). The leaf's own nodeId
    // (NodeModel.nodeId) is stable regardless of which stack members come
    // and go — resolve on that instead, everywhere in this component.
    const getOwnNode = () => findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);

    // Reads the SAME reactive field `pane-leaf-chrome.tsx`'s inner `<Key>`
    // is keyed on — see NodeModel.activeBlockId's own doc comment
    // (layout/lib/types.ts).
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? anchorBlockId;

    // A tab of this strip is a member of this pane's own block-stack, so
    // activating one swaps the active member in place. A block of another pane
    // is never offered here: one agent block is one pane, and drawing another
    // pane's block as a tab made two panes look like one group (#27069, #23913).
    const handleTabSwitch = (targetBlockId: string) => {
        if (targetBlockId === activeBlockId()) return;
        const node = getOwnNode();
        if (!node) return;
        const stack = node.data?.blockStack?.length ? node.data.blockStack : [activeBlockId()];
        // No reveal gate: AgentPaneChrome is mounted for the whole pane's life and never remounts on a
        // switch; only the inner <Block> rebuilds, and that is already covered by its own ready-gate
        // cross-fade. See SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
        if (stack.includes(targetBlockId)) setActiveBlockInStack(layoutModel, node.id, targetBlockId);
    };
    // × on a tab (also middle-click, via PaneTabStrip's onMouseDown).
    // closeAgentTab closes the block like closeBlockInStack does (pop it out
    // and activate the neighbor; last member closes the pane), with one exception: the last
    // tab of THIS pane holding a loaded agent is swapped for a fresh My
    // Agents tab instead of closing the pane
    // (SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §2).
    //
    // No reveal gate on the ordinary close paths — same reasoning as
    // handleTabSwitch: that node's own AgentPaneChrome (if it's an agent
    // pane) is mounted for the pane's whole life and never remounts on a
    // switch, so a gate would only hide content already covered by its own
    // ready-gate cross-fade. See SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
    // The picker swap holds its own gate (see close-agent-tab.ts).
    const handleTabClose = (targetBlockId: string) => {
        void closeAgentTab({ layoutModel, ownNodeId: nodeModel.nodeId, blockId: targetBlockId });
    };
    // No progress-bar slot here: the shared chrome renders it on every pane
    // and hands it to whichever view model is active (PaneChrome.tsx), so an
    // agent tab in a pane that started as another view type gets it too.
    return {
        // Both return true ("handled"): the strip's tabs are this pane's own stack members.
        onActivate: (id: string) => {
            handleTabSwitch(id);
            return true;
        },
        onClose: (id: string) => {
            handleTabClose(id);
            return true;
        },
        rootClass: "agent-pane-stack",
        contentClass: "agent-pane-stack-content",
    };
}
