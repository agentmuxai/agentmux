// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneChrome — a single, shared `ViewModel.renderPaneChrome`
 * implementation for every widget type that does NOT need agent's or
 * term's own richer, domain-specific chrome (fork-lineage tab merging,
 * rename-via-definition-API, per-tab zoom, etc.). Every OTHER widget type
 * (browser, editor, sysinfo, swarm, armory, media, drone, help, warden)
 * registers this SAME function rather than each growing its own
 * near-identical Chrome component — see each ViewModel's own one-line
 * `this.renderPaneChrome = renderPaneChromeShell` registration.
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
import { blockViewToIcon, blockViewToName, getBlockHeaderIcon } from "@/app/block/blockutil";
import { computeFocusRingBorderColor } from "@/app/block/blockframe";
import { atoms, MOS } from "@/app/store/global";
import { ErrorBoundary } from "@/element/errorboundary";
import { closeBlockInStack, getLayoutModelForStaticTab, setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import "./PaneChrome.scss";
import { openPaneTabWidgetPicker } from "./pane-tab-picker";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";

interface GenericPaneTab {
    blockId: string;
    label: string;
    icon: JSX.Element;
}

export function renderPaneChromeShell(nodeModel: NodeModel, content: JSX.Element): JSX.Element {
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
            // Reactive read (getMuxObjectAtom, not getObjectValue's plain
            // snapshot) — matches activeBlockData()'s own convention above.
            // ReAgent P1: a background tab's own meta (rename, frame:title/
            // frame:icon update) must re-run this memo too, not just the
            // active member's.
            const bd = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))();
            const label = (bd?.meta?.["frame:title"] as string | undefined) ?? blockViewToName(bd?.meta?.view);
            // Same icon convention the plain (non-tabbed) header iconview
            // uses (blockframe.tsx's viewIconElem) — derived from the
            // block's own persisted meta, not a live ViewModel, since a
            // background (non-active) tab's ViewModel isn't mounted here.
            const icon = getBlockHeaderIcon(
                (bd?.meta?.["frame:icon"] as string | undefined) ?? blockViewToIcon(bd?.meta?.view),
                bd
            );
            return { blockId, label, icon };
        });
    });

    // The active ViewModel's opted-in capabilities, resolved ONCE here (not
    // in a memo): `pane-leaf-chrome.tsx` latches the chrome ViewModel for the
    // pane's whole life, so re-deriving per switch would both contradict that
    // and re-create any signals the model builds. A view type that opts out
    // entirely (the nine that never implement it) leaves this null and gets
    // every default below unchanged.
    const model: PaneChromeModel | null = nodeModel.activeViewModel?.()?.paneChromeModel?.(nodeModel) ?? null;

    const handleActivate = (blockId: string) => {
        // A view type whose tabs can live in OTHER panes (agent's cross-pane
        // forks) handles activation itself and returns true; anything else
        // falls through to the ordinary same-pane stack switch.
        if (model?.onActivate?.(blockId) === true) return;
        if (blockId === activeBlockId()) return;
        setActiveBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    };
    const handleClose = (blockId: string) => {
        if (model?.onClose?.(blockId) === true) return;
        void closeBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    };
    const handleAdd = (e?: MouseEvent) => {
        if (!e) return;
        openPaneTabWidgetPicker(layoutModel, nodeModel.nodeId, e, model?.newTabMeta);
    };

    const activeViewModelOrUndefined = () => nodeModel.activeViewModel?.() ?? undefined;

    const renderHeader = (viewModel: ViewModel | null): JSX.Element => (
        <PaneHeaderTabStrip
            tabs={model?.tabs?.() ?? tabs()}
            activeId={activeBlockId()}
            getId={model?.getId ?? ((t: any) => t.blockId)}
            getLabel={model?.getLabel ?? ((t: any) => t.label)}
            getIcon={model?.getIcon ?? ((t: any) => t.icon)}
            getTooltip={model?.getTooltip}
            getAttention={model?.getAttention}
            getTabClass={model?.getTabClass}
            onActivate={handleActivate}
            onClose={handleClose}
            onTabDoubleClick={model?.onTabDoubleClick}
            renderLabel={model?.renderLabel}
            zoomFactor={model?.zoomFactor}
            connBtnRef={model?.connBtnRef}
            changeConnModalAtom={model?.changeConnModalAtom}
            onAdd={handleAdd}
            addTitle={model?.addTitle ?? "Add tab"}
            nodeModel={nodeModel}
            viewModel={viewModel}
            activeBlockId={activeBlockId}
        />
    );

    const contentRegion = <div class={model?.contentClass ?? "pane-stack-content"}>{content}</div>;

    return (
        <div
            class="pane-stack"
            classList={{
                "pane-stack-focused": isFocused() && !isAlone(),
                "pane-stack-focused-alone": isFocused() && isAlone(),
                ...(model?.rootClass ? { [model.rootClass]: true } : {}),
            }}
            style={{ "--pane-ring-color": ringBorderColor() }}
            data-blockid={activeBlockId()}
            onClick={() => nodeModel.focusNode()}
            onFocusIn={() => nodeModel.focusNode()}
        >
            <ErrorBoundary fallback={renderHeader(null)}>
                {renderHeader(activeViewModelOrUndefined() ?? null)}
            </ErrorBoundary>
            {model?.renderBelowHeader?.()}
            {model?.wrapContent ? model.wrapContent(contentRegion) : contentRegion}
        </div>
    );
}
