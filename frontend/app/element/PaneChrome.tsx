// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneChrome — the ONE `ViewModel.renderPaneChrome` implementation, shared
 * by every pane type. It owns the pane's tab list: every stack member (plus
 * any `PaneChromeModel.extraTabs`) is described through `describePaneTab`
 * (pane-tab-model.tsx), so labels, icons and rename behave identically for
 * agent, terminal and every widget type. A view type customises that only
 * by registering a `PaneTabDescriptor`, and customises the chrome around the
 * tabs through `PaneChromeModel`.
 *
 * Every pane shows its tabs as pills, including a lone tab, so a tab's icon
 * and size never change as tabs are added or closed. The pills scale only
 * with chrome zoom (the header row's `--zoomfactor`).
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
 * §4.1/§4.5.
 */

import { Show, createEffect, createMemo, type JSX } from "solid-js";
import { UnseenTurnsDot } from "@/app/block/UnseenTurnsDot";
import { computeFocusRingBorderColor } from "@/app/block/blockframe";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { atoms, MOS } from "@/app/store/global";
import { remoteDisplay } from "@/app/store/remote-display";
import { ErrorBoundary } from "@/element/errorboundary";
import { isBlockClosing } from "@/layout/lib/layoutClosing";
import type { NodeModel } from "@/layout/index";
import { paneTabSelectionApi } from "./pane-tab-selection";
import "./PaneChrome.scss";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";
import { PaneTabIconView } from "./pane-tab-model";
import { PaneTabRenameInput } from "./PaneTabRenameInput";
import { usePaneTabProgress } from "@/app/store/pane-tab-progress";
import { DropIndicator } from "@/app/drag/DropIndicator";
import { paneDropState } from "@/app/drag/file-drop";
import {
    createHeaderTailBg,
    createPaneChromeModel,
    createPaneTabDrops,
    createPaneTabHandlers,
    createStackIds,
    createTabColors,
    createTabIds,
    createTabInfos,
    createTabRename,
    tabSeatOf,
    tabSummaryOf,
    usePaneLocateFlash,
    usePaneStripReveal,
    usePaneTabSelectionPruning,
    useProgressBarSlot,
} from "./pane-chrome-hooks";

export function renderPaneChromeShell(nodeModel: NodeModel, content: JSX.Element): JSX.Element {
    // `nodeModel.layoutModel`, NOT `getLayoutModelForStaticTab()`: the tab this pane's own leaf lives
    // in is not necessarily whichever tab is globally active when the chrome first constructs
    // (SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md).
    const layoutModel = nodeModel.layoutModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());
    const isFocused = () => nodeModel.isFocused();
    const stackIds = createStackIds(nodeModel);

    let paneRoot: HTMLDivElement | undefined;
    const getPaneRoot = () => paneRoot;
    usePaneStripReveal(isFocused, getPaneRoot);
    usePaneLocateFlash(nodeModel, isFocused, activeBlockId, stackIds, getPaneRoot);

    const isAlone = () => nodeModel.numLeafs() <= 1;
    // A stripe in the remote's colour when the active tab is on one (SPEC_REMOTES_PANE_2026_10_05.md §4.8).
    const remoteColor = createMemo(
        () => remoteDisplay(atoms.fullConfigAtom()?.connections, activeBlockData()?.meta?.connection)?.color
    );
    // Each tab on a remote with a colour shows it as a swatch beside its label.
    const remoteSwatchOf = (blockId: string): string | undefined =>
        remoteDisplay(
            atoms.fullConfigAtom()?.connections,
            MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta?.connection
        )?.color;
    const ringBorderColor = createMemo(() => {
        const isLightTheme = isLightThemeActive();
        return computeFocusRingBorderColor(isFocused(), activeBlockData()?.meta, isLightTheme);
    });

    const model = createPaneChromeModel(nodeModel, activeBlockId, activeBlockData);
    // Tabs are keyed by blockId strings, so PaneTabStrip's <For> keeps each
    // pill's DOM node across recomputes; label/icon are looked up per id.
    const extraTabs = createMemo(() => model()?.extraTabs?.() ?? []);
    const tabIds = createTabIds(stackIds, extraTabs);
    const tabInfos = createTabInfos(nodeModel, stackIds, tabIds, extraTabs, activeBlockId);
    const tabColors = createTabColors(tabIds);
    const headerTailBg = createHeaderTailBg(tabIds);
    const { renamingId, setRenamingId, labelOf, confirmRename } = createTabRename(tabInfos);
    const summaryOf = (id: string) => tabSummaryOf(id, tabInfos);
    const handlers = createPaneTabHandlers(nodeModel, model, stackIds, activeBlockId);
    const drops = createPaneTabDrops(nodeModel, stackIds, activeBlockId);
    usePaneTabSelectionPruning(layoutModel);

    const activeViewModelOrUndefined = () => nodeModel.activeViewModel?.() ?? undefined;
    const setProgressSlotEl = useProgressBarSlot(nodeModel);

    let bodyEl: HTMLDivElement | undefined;
    createEffect(() => {
        const closing = nodeModel.isClosing?.() ?? false;
        if (bodyEl) bodyEl.inert = closing;
    });

    const renderHeader = (viewModel: ViewModel | null): JSX.Element => (
        <PaneHeaderTabStrip
            tabs={tabIds()}
            activeId={activeBlockId()}
            getId={(id) => id}
            getLabel={labelOf}
            getTooltipDetail={summaryOf}
            getTooltipSeat={tabSeatOf}
            getIcon={(id) => <PaneTabIconView icon={() => tabInfos().get(id)?.icon} badge={() => tabInfos().get(id)?.badge} />}
            getColor={(id) => tabColors().get(id)}
            useProgress={usePaneTabProgress}
            collapsed={() => nodeModel.isMinimized?.() ?? false}
            isClosing={(id) => (nodeModel.isClosing?.() ?? false) || isBlockClosing(layoutModel, id)}
            onTabContextMenu={handlers.handleTabContextMenu}
            headerBgOverride={headerTailBg()}
            onActivate={handlers.handleActivate}
            onSelect={handlers.handleSelect}
            onClose={handlers.handleClose}
            onReorder={handlers.handleReorder}
            onReceiveForeignTab={drops.handleReceiveForeignTab}
            onReceiveForeignTabs={drops.handleReceiveForeignTabs}
            acceptsForeignTabs={drops.acceptsForeignTabs}
            bodyDrop={drops.bodyDrop}
            selection={paneTabSelectionApi}
            onTabDoubleClick={(id) => tabInfos().get(id)?.rename && setRenamingId(id)}
            renderLabel={(id) =>
                renamingId() === id ? (
                    <PaneTabRenameInput
                        initialValue={labelOf(id)}
                        onConfirm={(title) => void confirmRename(id, title)}
                        onCancel={() => setRenamingId(null)}
                    />
                ) : (
                    <>
                        <Show when={remoteSwatchOf(id)}>
                            {(color) => <span class="pane-tab-swatch" style={{ background: color() }} />}
                        </Show>
                        <span class="pane-tab-label">{labelOf(id)}</span>
                        {/* Turns that finished unseen in this tab: on its own
                            pill, so it stays visible after switching tabs. */}
                        <UnseenTurnsDot blockId={id} />
                    </>
                )
            }
            connBtnRef={model()?.connBtnRef}
            changeConnModalAtom={model()?.changeConnModalAtom}
            onAdd={handlers.handleAdd}
            addTitle={model()?.addTitle ?? "Add tab"}
            nodeModel={nodeModel}
            viewModel={viewModel}
            activeBlockId={activeBlockId}
        />
    );

    // Stable elements: only their classes and the body's leading overlays
    // follow the active tab's model, so a switch between view types never
    // moves the (kept-alive) content in the DOM.
    const contentRegion = <div class={model()?.contentClass ?? "pane-stack-content"}>{content}</div>;
    // File drop feedback for this pane (SPEC_DRAG_AND_DROP_CONSOLIDATION §4).
    const dropState = paneDropState(activeBlockId);

    return (
        <div
            class="pane-stack"
            ref={(el) => {
                paneRoot = el;
            }}
            // The whole-pane drop zone for a Pane Tab dragged from another
            // pane (PaneTabStrip's `foreignDropRootFor`).
            data-role="pane"
            classList={{
                "pane-stack-focused": isFocused() && !isAlone(),
                "pane-stack-focused-alone": isFocused() && isAlone(),
                "pane-stack-remote-color": !!remoteColor(),
                // A collapsed pane shows no border animation, whichever tab is selected: its
                // pills' progress lines say who works (PaneChrome.scss, SPEC_PANE_TAB_PROGRESS_LINE §3.4).
                "pane-stack-collapsed": nodeModel.isMinimized?.() ?? false,
                // From the click on X until the pane is gone: dimmed and inert, "Closing…" in the header.
                "pane-stack-closing": nodeModel.isClosing?.() ?? false,
                ...(model()?.rootClass ? { [model()!.rootClass]: true } : {}),
            }}
            style={{ "--pane-ring-color": ringBorderColor(), "--pane-remote-color": remoteColor() }}
            data-blockid={activeBlockId()}
            data-file-drop={dropState()?.state}
            onClick={() => nodeModel.focusNode()}
            onFocusIn={() => nodeModel.focusNode()}
        >
            <ErrorBoundary fallback={renderHeader(null)}>
                {renderHeader(activeViewModelOrUndefined() ?? null)}
            </ErrorBoundary>
            {model()?.renderBelowHeader?.()}
            <div class="pane-progress-bar-slot" ref={setProgressSlotEl} />
            {/* `inert`, not only pointer-events: the kept-alive content slot takes pointer events back
                and a focused field would still take keys, for a pane that is about to be deleted. */}
            <div class={`pane-stack-body${model()?.bodyClass ? ` ${model()!.bodyClass}` : ""}`} ref={(el) => (bodyEl = el)}>
                {model()?.renderBehindContent?.()}
                {contentRegion}
            </div>
            <Show when={dropState()}>{(s) => <DropIndicator state={s()} />}</Show>
        </div>
    );
}
