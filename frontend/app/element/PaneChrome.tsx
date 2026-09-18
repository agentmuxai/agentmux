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

import { createMemo, createSignal, type JSX } from "solid-js";
import { computeFocusRingBorderColor } from "@/app/block/blockframe";
import { atoms, MOS, pushNotification } from "@/app/store/global";
import { ErrorBoundary } from "@/element/errorboundary";
import { closeBlockInStack, setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import "./PaneChrome.scss";
import { openPaneTabWidgetPicker } from "./pane-tab-picker";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";
import { createPaneTabMemory, describePaneTab, PaneTabIconView, prunePaneTabMemory, type PaneTabInfo } from "./pane-tab-model";
import { PaneTabRenameInput } from "./PaneTabRenameInput";

function sameIds(a: string[], b: string[]): boolean {
    return a.length === b.length && a.every((id, i) => id === b[i]);
}

export function renderPaneChromeShell(nodeModel: NodeModel, content: JSX.Element): JSX.Element {
    // `nodeModel.layoutModel`, NOT `getLayoutModelForStaticTab()` — the tab
    // this pane's own leaf lives in is not necessarily "whichever tab is
    // globally active right now" at the moment chrome first constructs. See
    // that field's own doc comment (layout/lib/types.ts) and
    // SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md.
    const layoutModel = nodeModel.layoutModel;
    const getOwnNode = () => findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());

    const isFocused = () => nodeModel.isFocused();
    const isAlone = () => nodeModel.numLeafs() <= 1;
    const ringBorderColor = createMemo(() =>
        computeFocusRingBorderColor(isFocused(), activeBlockData()?.meta, atoms.tabAtom()?.meta)
    );

    // The active ViewModel's opted-in capabilities, resolved ONCE here (not
    // in a memo): `pane-leaf-chrome.tsx` latches the chrome ViewModel for the
    // pane's whole life, so re-deriving per switch would both contradict that
    // and re-create any signals the model builds. A view type that opts out
    // entirely (the nine that never implement it) leaves this null and gets
    // every default below unchanged.
    const model: PaneChromeModel | null = nodeModel.activeViewModel?.()?.paneChromeModel?.(nodeModel) ?? null;

    // Tabs are keyed by blockId strings, so PaneTabStrip's <For> keeps each
    // pill's DOM node across recomputes; label/icon are looked up per id.
    const stackIds = createMemo<string[]>(
        () => {
            layoutModel.localTreeStateAtom();
            const stack = getOwnNode()?.data?.blockStack;
            return stack?.length ? [...stack] : [activeBlockId()];
        },
        undefined,
        { equals: sameIds }
    );
    const extraTabs = createMemo(() => model?.extraTabs?.() ?? []);
    const tabIds = createMemo<string[]>(
        () => {
            const stack = stackIds();
            const inStack = new Set(stack);
            return [...stack, ...extraTabs().map((t) => t.blockId).filter((id) => !inStack.has(id))];
        },
        undefined,
        { equals: sameIds }
    );
    const tabMemory = createPaneTabMemory();
    const tabInfos = createMemo(() => {
        const inStack = new Set(stackIds());
        const extraLabels = new Map(extraTabs().map((t) => [t.blockId, t.label]));
        const activeId = activeBlockId();
        const liveVm = nodeModel.activeViewModel?.() ?? null;
        const ordinals = new Map<string, number>();
        const infos = new Map<string, PaneTabInfo>();
        for (const blockId of tabIds()) {
            // Reactive read: a background tab's own meta changes (rename,
            // agent launch, frame:icon) must update its pill too.
            const meta = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta;
            const view = meta?.view as string | undefined;
            let ordinal = 0;
            if (inStack.has(blockId)) {
                ordinal = (ordinals.get(view ?? "") ?? 0) + 1;
                ordinals.set(view ?? "", ordinal);
            }
            infos.set(
                blockId,
                describePaneTab(
                    { blockId, view, meta, ordinal, liveViewModel: blockId === activeId ? liveVm : null },
                    extraLabels.get(blockId),
                    tabMemory
                )
            );
        }
        prunePaneTabMemory(tabMemory, infos.keys());
        return infos;
    });

    // Rename (double-click a pill). The override shows the new name at once,
    // before the write round-trips back through the block's meta.
    const [renamingId, setRenamingId] = createSignal<string | null>(null);
    const [titleOverrides, setTitleOverrides] = createSignal<Record<string, string>>({});
    const labelOf = (id: string) => titleOverrides()[id] ?? tabInfos().get(id)?.label ?? "";
    const confirmRename = async (id: string, title: string) => {
        setRenamingId(null);
        const rename = tabInfos().get(id)?.rename;
        if (!rename) return;
        const prev = titleOverrides()[id];
        setTitleOverrides((o) => ({ ...o, [id]: title }));
        try {
            await rename(title);
        } catch (e: unknown) {
            setTitleOverrides((o) => {
                const next = { ...o };
                if (prev === undefined) delete next[id];
                else next[id] = prev;
                return next;
            });
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Rename failed",
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
        }
    };

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
            tabs={tabIds()}
            activeId={activeBlockId()}
            getId={(id) => id}
            getLabel={labelOf}
            getIcon={(id) => <PaneTabIconView icon={() => tabInfos().get(id)?.icon} />}
            onActivate={handleActivate}
            onClose={handleClose}
            onTabDoubleClick={(id) => tabInfos().get(id)?.rename && setRenamingId(id)}
            renderLabel={(id) =>
                renamingId() === id ? (
                    <PaneTabRenameInput
                        initialValue={labelOf(id)}
                        onConfirm={(title) => void confirmRename(id, title)}
                        onCancel={() => setRenamingId(null)}
                    />
                ) : (
                    <span class="pane-tab-label">{labelOf(id)}</span>
                )
            }
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
