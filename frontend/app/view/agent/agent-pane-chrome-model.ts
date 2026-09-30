// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 1).

import { MOS } from "@/app/store/global";
import { revealBlockLocally } from "@/app/util/reveal-block";
import { setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import { createMemo } from "solid-js";
import { agentModels } from "./agent-models";
import { closeAgentTab } from "./close-agent-tab";
import { useOpenDefinitionMap } from "./components/AgentPicker";
import { useForkSet } from "./fork/useForkSet";

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
 * its tab model (fork lineage merged with this pane's own stack, rename,
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

    // Block-scoped reads (agentId) must track the
    // CURRENTLY ACTIVE member, not `anchorBlockId` (frozen to whichever
    // ViewModel instance first rendered this chrome) — getMuxObjectAtom
    // inside a memo, not useMuxObjectValue, the same reactive-oref pattern
    // PR #3134 already established for BlockFrame_Header
    // (frontend/app/store/mos.ts's own doc comments explain why).
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());
    const agentId = () => activeBlockData()?.meta?.["agentId"];

    // Fork tabs: conversations sharing this one's `parent_id` lineage that
    // are open in ANOTHER top-level pane, shown as extra pills (PaneChrome
    // dedupes them against this pane's own stack) so cross-pane
    // fork-switching keeps working.
    const [openDefinitions] = useOpenDefinitionMap();
    // ReAgent P2: reads the active tab's OWN agentDefinitions
    // (agent-model.ts) instead of calling useAgentDefinitions() again here
    // — that would be a second, independent RPC + agents:changed
    // subscription for the same pane, on top of the one AgentBlockContent
    // already owns. Found by block id (agent-models.ts): the host's view
    // model is only an adapter once the agent is a native pane tab.
    const forks = useForkSet({
        definitions: () => agentModels.get(activeBlockId())?.agentDefinitions() ?? [],
        openBlockByDef: openDefinitions,
        activeDefinitionId: () => agentId() ?? "",
    });
    // Same "switchable" filter AgentPresentationView used to apply: a fork
    // with no open blockId anywhere can't be jumped to, so it isn't offered.
    const switchableForks = createMemo(() => forks().filter((f) => f.isActive || !!f.blockId));

    // Forks open in ANOTHER pane appear as extra pills after this pane's own
    // stack members, labeled with their branch/definition title.
    const extraTabs = () =>
        switchableForks()
            .filter((f) => !!f.blockId)
            .map((f) => ({ blockId: f.blockId!, label: f.title }));

    // Activating a tab has two cases, both "switch," neither "create": (1)
    // the target block already lives in THIS pane's own block-stack — swap
    // the active member in place; (2) a fork open as its own separate
    // top-level pane — reveal it via revealBlockLocally, same as the
    // picker's "Switch to existing" flow already does.
    const handleTabSwitch = (targetBlockId: string) => {
        if (targetBlockId === activeBlockId()) return;
        const node = getOwnNode();
        if (!node) return;
        const stack = node.data?.blockStack?.length ? node.data.blockStack : [activeBlockId()];
        if (stack.includes(targetBlockId)) {
            // No reveal gate: reaching this branch at all requires
            // node.data.blockStack.length > 1 (otherwise `stack` above is
            // just [activeBlockId()], and targetBlockId !== activeBlockId()
            // already ruled out targetBlockId matching it) — the
            // precondition for a second pill to exist to click at all.
            // AgentPaneChrome is mounted for the whole pane's life and
            // never remounts on a switch, so there is nothing for a gate to
            // hide: only the inner <Block> rebuilds, and that is already
            // covered by its own ready-gate cross-fade.
            // See SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
            setActiveBlockInStack(layoutModel, node.id, targetBlockId);
        } else {
            // Another pane — switch it to the fork if it's a background tab
            // there (refocusNode only focused the pane).
            void revealBlockLocally(targetBlockId);
        }
    };
    // × on a tab (also middle-click, via PaneTabStrip's onMouseDown).
    // closeAgentTab resolves the block's OWNING node — for a stack member
    // that's this pane, for a cross-pane fork tab it's that other pane — and
    // closes it like closeBlockInStack does (pop it out and activate the
    // neighbor; last member closes the pane), with one exception: the last
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
        extraTabs,
        // Both return true ("handled"): an agent tab may live in a
        // DIFFERENT pane (a fork open as its own top-level pane), so
        // activating/closing it isn't necessarily this pane's own stack
        // operation — handleTabSwitch/handleTabClose resolve the owning
        // node themselves.
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
