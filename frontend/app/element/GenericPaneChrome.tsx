// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GenericPaneChrome — a single, shared `ViewModel.renderPaneChrome`
 * implementation for every widget type that does NOT need agent's or
 * term's own richer, domain-specific chrome (fork-lineage tab merging,
 * rename-via-definition-API, per-tab zoom, etc.). Every OTHER widget type
 * (browser, editor, sysinfo, swarm, armory, media, drone, help, warden)
 * registers this SAME function rather than each growing its own
 * near-identical Chrome component — see each ViewModel's own one-line
 * `this.renderPaneChrome = genericRenderPaneChrome` registration.
 *
 * Uses only the already view-agnostic primitives `layoutStack.ts` and
 * `action-widgets-config.ts` already provide: `blockStack` membership for
 * tabs, `setActiveBlockInStack`/`closeBlockInStack` for switch/close, and
 * `buildPaneWidgetMenuItems` + `addWidgetAsPaneTab` for "+" (the SAME
 * widget picker the widget bar's own right-click menu already builds).
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
 * §4.1/§4.5. Plan: docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md
 * Task Group C (extending Task Groups A/B's pattern to every remaining
 * widget type).
 */

import { createMemo, type JSX } from "solid-js";
import { blockViewToName } from "@/app/block/blockutil";
import { computeFocusRingBorderColor } from "@/app/block/blockframe";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { atoms, MOS } from "@/app/store/global";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { ErrorBoundary } from "@/element/errorboundary";
import { addWidgetAsPaneTab, closeBlockInStack, getLayoutModelForStaticTab, setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import "./GenericPaneChrome.scss";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";

interface GenericPaneTab {
    blockId: string;
    label: string;
}

export function genericRenderPaneChrome(nodeModel: NodeModel, content: JSX.Element): JSX.Element {
    const layoutModel = getLayoutModelForStaticTab();
    const getOwnNode = () => findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());

    const isFocused = () => nodeModel.isFocused();
    const isAlone = () => nodeModel.numLeafs() <= 1;
    const ringBorderColor = createMemo(() =>
        computeFocusRingBorderColor(isFocused(), activeBlockData()?.meta, atoms.tabAtom()?.meta)
    );

    // Only render pills once there's something to switch BETWEEN — matches
    // agent's/term's own "a lone tab shows no self-pill" convention
    // (visibleTabs()/visibleTermTabs()), not a new rule invented here.
    const tabs = createMemo<GenericPaneTab[]>(() => {
        layoutModel.localTreeStateAtom();
        const node = getOwnNode();
        const stack = node?.data?.blockStack?.length ? node.data.blockStack : [];
        if (stack.length <= 1) return [];
        return stack.map((blockId) => {
            const bd = MOS.getObjectValue(MOS.makeORef("block", blockId)) as Block | undefined;
            const label = (bd?.meta?.["frame:title"] as string | undefined) ?? blockViewToName(bd?.meta?.view);
            return { blockId, label };
        });
    });

    const handleActivate = (blockId: string) => {
        if (blockId === activeBlockId()) return;
        setActiveBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    };
    const handleClose = (blockId: string) => void closeBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    const handleAdd = (e?: MouseEvent) => {
        if (!e) return;
        const wmap = atoms.fullConfigAtom()?.widgets ?? {};
        const settings = atoms.fullConfigAtom()?.settings ?? {};
        const items = buildPaneWidgetMenuItems(wmap, settings, (blockDef) => {
            void addWidgetAsPaneTab(layoutModel, nodeModel.nodeId, blockDef);
        });
        ContextMenuModel.showContextMenu(items, e);
    };

    const activeViewModelOrUndefined = () => nodeModel.activeViewModel?.() ?? undefined;

    const renderHeader = (viewModel: ViewModel | null): JSX.Element => (
        <PaneHeaderTabStrip
            tabs={tabs()}
            activeId={activeBlockId()}
            getId={(t) => t.blockId}
            getLabel={(t) => t.label}
            onActivate={handleActivate}
            onClose={handleClose}
            onAdd={handleAdd}
            addTitle="Add tab"
            nodeModel={nodeModel}
            viewModel={viewModel}
            activeBlockId={activeBlockId}
        />
    );

    return (
        <div
            class="generic-pane-stack"
            classList={{
                "generic-pane-stack-focused": isFocused() && !isAlone(),
                "generic-pane-stack-focused-alone": isFocused() && isAlone(),
            }}
            style={{ "--pane-ring-color": ringBorderColor() }}
            data-blockid={activeBlockId()}
            onClick={() => nodeModel.focusNode()}
            onFocusIn={() => nodeModel.focusNode()}
        >
            <ErrorBoundary fallback={renderHeader(null)}>
                {renderHeader(activeViewModelOrUndefined() ?? null)}
            </ErrorBoundary>
            <div class="generic-pane-stack-content">{content}</div>
        </div>
    );
}
