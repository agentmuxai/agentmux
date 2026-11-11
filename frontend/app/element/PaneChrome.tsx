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

/** The pane's identity: the layout it lives in, its active block and focus, its stack, and its root and body elements. */
function createShellState(nodeModel: NodeModel) {
    // `nodeModel.layoutModel`, NOT `getLayoutModelForStaticTab()`: the tab this pane's own leaf lives
    // in is not necessarily whichever tab is globally active when the chrome first constructs
    // (SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md).
    const layoutModel = nodeModel.layoutModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());
    const isFocused = () => nodeModel.isFocused();
    const stackIds = createStackIds(nodeModel);

    const refs: { root?: HTMLDivElement; body?: HTMLDivElement } = {};
    const getPaneRoot = () => refs.root;
    usePaneStripReveal(isFocused, getPaneRoot);
    usePaneLocateFlash(nodeModel, isFocused, activeBlockId, stackIds, getPaneRoot);
    return { layoutModel, activeBlockId, activeBlockData, isFocused, stackIds, refs };
}
type ShellState = ReturnType<typeof createShellState>;

/** How the pane looks around its tabs: alone or not, the remote's stripe and swatches, the focus ring. */
function createShellLook(nodeModel: NodeModel, s: ShellState) {
    const isAlone = () => nodeModel.numLeafs() <= 1;
    // A stripe in the remote's colour when the active tab is on one (SPEC_REMOTES_PANE_2026_10_05.md §4.8).
    const remoteColor = createMemo(
        () => remoteDisplay(atoms.fullConfigAtom()?.connections, s.activeBlockData()?.meta?.connection)?.color
    );
    // Each tab on a remote with a colour shows it as a swatch beside its label.
    const remoteSwatchOf = (blockId: string): string | undefined =>
        remoteDisplay(
            atoms.fullConfigAtom()?.connections,
            MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta?.connection
        )?.color;
    const ringBorderColor = createMemo(() => {
        const isLightTheme = isLightThemeActive();
        return computeFocusRingBorderColor(s.isFocused(), s.activeBlockData()?.meta, isLightTheme);
    });
    return { isAlone, remoteColor, remoteSwatchOf, ringBorderColor };
}
type ShellLook = ReturnType<typeof createShellLook>;

/** The pane's tabs: the chrome model, their ids, labels, colours and rename, and what the strip does with them. */
function createShellTabs(nodeModel: NodeModel, s: ShellState) {
    const model = createPaneChromeModel(nodeModel, s.activeBlockId, s.activeBlockData);
    // Tabs are keyed by blockId strings, so PaneTabStrip's <For> keeps each
    // pill's DOM node across recomputes; label/icon are looked up per id.
    const extraTabs = createMemo(() => model()?.extraTabs?.() ?? []);
    const tabIds = createTabIds(s.stackIds, extraTabs);
    const tabInfos = createTabInfos(nodeModel, s.stackIds, tabIds, extraTabs, s.activeBlockId);
    const tabColors = createTabColors(tabIds);
    const headerTailBg = createHeaderTailBg(tabIds);
    const { renamingId, setRenamingId, labelOf, confirmRename } = createTabRename(tabInfos);
    const summaryOf = (id: string) => tabSummaryOf(id, tabInfos);
    const handlers = createPaneTabHandlers(nodeModel, model, s.stackIds, s.activeBlockId);
    const drops = createPaneTabDrops(nodeModel, s.stackIds, s.activeBlockId);
    usePaneTabSelectionPruning(s.layoutModel);
    return { model, tabIds, tabInfos, tabColors, headerTailBg, renamingId, setRenamingId, labelOf, confirmRename, summaryOf, handlers, drops };
}
type ShellTabs = ReturnType<typeof createShellTabs>;

/** What a tab shows as its label: the rename input while it is renamed, else its swatch, name and unseen-turns dot. */
function renderTabLabel(id: string, look: ShellLook, t: ShellTabs): JSX.Element {
    return t.renamingId() === id ? (
        <PaneTabRenameInput
            initialValue={t.labelOf(id)}
            onConfirm={(title) => void t.confirmRename(id, title)}
            onCancel={() => t.setRenamingId(null)}
        />
    ) : (
        <>
            <Show when={look.remoteSwatchOf(id)}>{(color) => <span class="pane-tab-swatch" style={{ background: color() }} />}</Show>
            <span class="pane-tab-label">{t.labelOf(id)}</span>
            {/* Turns that finished unseen in this tab: on its own
                pill, so it stays visible after switching tabs. */}
            <UnseenTurnsDot blockId={id} />
        </>
    );
}

function renderShellHeader(
    nodeModel: NodeModel,
    s: ShellState,
    look: ShellLook,
    t: ShellTabs,
    viewModel: ViewModel | null
): JSX.Element {
    return (
        <PaneHeaderTabStrip
            tabs={t.tabIds()}
            activeId={s.activeBlockId()}
            getId={(id) => id}
            getLabel={t.labelOf}
            getTooltipDetail={t.summaryOf}
            getTooltipSeat={tabSeatOf}
            getTooltipModel={(id) => t.tabInfos().get(id)?.modelLine}
            getIcon={(id) => <PaneTabIconView icon={() => t.tabInfos().get(id)?.icon} badge={() => t.tabInfos().get(id)?.badge} />}
            getColor={(id) => t.tabColors().get(id)}
            useProgress={usePaneTabProgress}
            collapsed={() => nodeModel.isMinimized?.() ?? false}
            isClosing={(id) => (nodeModel.isClosing?.() ?? false) || isBlockClosing(s.layoutModel, id)}
            onTabContextMenu={t.handlers.handleTabContextMenu}
            headerBgOverride={t.headerTailBg()}
            onActivate={t.handlers.handleActivate}
            onSelect={t.handlers.handleSelect}
            onClose={t.handlers.handleClose}
            onReorder={t.handlers.handleReorder}
            onReceiveForeignTab={t.drops.handleReceiveForeignTab}
            onReceiveForeignTabs={t.drops.handleReceiveForeignTabs}
            acceptsForeignTabs={t.drops.acceptsForeignTabs}
            bodyDrop={t.drops.bodyDrop}
            selection={paneTabSelectionApi}
            onTabDoubleClick={(id) => t.tabInfos().get(id)?.rename && t.setRenamingId(id)}
            renderLabel={(id) => renderTabLabel(id, look, t)}
            connBtnRef={t.model()?.connBtnRef}
            changeConnModalAtom={t.model()?.changeConnModalAtom}
            onAdd={t.handlers.handleAdd}
            addTitle={t.model()?.addTitle ?? "Add tab"}
            nodeModel={nodeModel}
            viewModel={viewModel}
            activeBlockId={s.activeBlockId}
        />
    );
}

export function renderPaneChromeShell(nodeModel: NodeModel, content: JSX.Element): JSX.Element {
    const s = createShellState(nodeModel);
    const look = createShellLook(nodeModel, s);
    const t = createShellTabs(nodeModel, s);

    const activeViewModelOrUndefined = () => nodeModel.activeViewModel?.() ?? undefined;
    const setProgressSlotEl = useProgressBarSlot(nodeModel);

    createEffect(() => {
        const closing = nodeModel.isClosing?.() ?? false;
        if (s.refs.body) s.refs.body.inert = closing;
    });

    const renderHeader = (viewModel: ViewModel | null): JSX.Element => renderShellHeader(nodeModel, s, look, t, viewModel);

    // Stable elements: only their classes and the body's leading overlays
    // follow the active tab's model, so a switch between view types never
    // moves the (kept-alive) content in the DOM.
    const contentRegion = <div class={t.model()?.contentClass ?? "pane-stack-content"}>{content}</div>;
    // File drop feedback for this pane (SPEC_DRAG_AND_DROP_CONSOLIDATION §4).
    const dropState = paneDropState(s.activeBlockId);

    return (
        <div
            class="pane-stack"
            ref={(el) => {
                s.refs.root = el;
            }}
            // The whole-pane drop zone for a Pane Tab dragged from another
            // pane (PaneTabStrip's `foreignDropRootFor`).
            data-role="pane"
            classList={{
                "pane-stack-focused": s.isFocused() && !look.isAlone(),
                "pane-stack-focused-alone": s.isFocused() && look.isAlone(),
                "pane-stack-remote-color": !!look.remoteColor(),
                // A collapsed pane shows no border animation, whichever tab is selected: its
                // pills' progress lines say who works (PaneChrome.scss, SPEC_PANE_TAB_PROGRESS_LINE §3.4).
                "pane-stack-collapsed": nodeModel.isMinimized?.() ?? false,
                // From the click on X until the pane is gone: dimmed and inert, "Closing…" in the header.
                "pane-stack-closing": nodeModel.isClosing?.() ?? false,
                ...(t.model()?.rootClass ? { [t.model()!.rootClass]: true } : {}),
            }}
            style={{ "--pane-ring-color": look.ringBorderColor(), "--pane-remote-color": look.remoteColor() }}
            data-blockid={s.activeBlockId()}
            data-file-drop={dropState()?.state}
            onClick={() => nodeModel.focusNode()}
            onFocusIn={() => nodeModel.focusNode()}
        >
            <ErrorBoundary fallback={renderHeader(null)}>
                {renderHeader(activeViewModelOrUndefined() ?? null)}
            </ErrorBoundary>
            {t.model()?.renderBelowHeader?.()}
            <div class="pane-progress-bar-slot" ref={setProgressSlotEl} />
            {/* `inert`, not only pointer-events: the kept-alive content slot takes pointer events back
                and a focused field would still take keys, for a pane that is about to be deleted. */}
            <div class={`pane-stack-body${t.model()?.bodyClass ? ` ${t.model()!.bodyClass}` : ""}`} ref={(el) => (s.refs.body = el)}>
                {t.model()?.renderBehindContent?.()}
                {contentRegion}
            </div>
            <Show when={dropState()}>{(d) => <DropIndicator state={d()} />}</Show>
        </div>
    );
}
