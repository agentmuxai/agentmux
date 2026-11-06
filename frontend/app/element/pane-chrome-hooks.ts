// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The reactive parts of `renderPaneChromeShell` (PaneChrome.tsx), one function per concern: the
 * strip reveal and locate flash on the pane root, the per-view chrome model, the tab descriptions,
 * colors and header tail, rename, the pill handlers and the drops. Each is called from the shell in
 * the order the shell lists them, in the shell's own reactive scope.
 */

import { createEffect, createMemo, createSignal, getOwner, on, onCleanup, onMount, runWithOwner, untrack, type Accessor } from "solid-js";
import {
    computeBlockColorBg,
    computeBlockIdentityColor,
    computeBlockTabPillActiveBg,
    computeBlockTabPillBg,
    computeBlockTabPillNeutralBg,
    computeMixedPaneHeaderBg,
    showPaneTabMenu,
} from "@/app/block/blockframe";
import { getPaneTab, paneTabCapability, resolvePaneTabView } from "@/app/store/pane-tab-registry";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { MOS, pushNotification } from "@/app/store/global";
import { shouldReduceMotion } from "@/app/motion/motion";
import { readSwarmSummary } from "@/app/store/activitySummary";
import { readSeatMeta, seatTooltipLine } from "@/app/agent-model/seat-format";
import {
    closeBlockInStack,
    mergeableDocumentPanes,
    mergeDocumentPanesInStack,
    moveBlockInStack,
    moveBlocksInStack,
    setActiveBlockInStack,
    type NodeModel,
} from "@/layout/index";
import { placePill, type PillZone } from "@/layout/lib/paneBodyDrop";
import { paneStackIds } from "@/layout/lib/layoutNodeModels";
import { allBlockIds } from "@/layout/lib/stackOrder";
import { clampCrossTabDirection } from "@/layout/lib/crossTabDrag";
import { foreignTabMoves } from "./pane-foreign-tab";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import { clearPaneTabSelection, paneTabSelection, prunePaneTabSelection } from "./pane-tab-selection";
import { openPaneTabWidgetPicker } from "./pane-tab-picker";
import { installPaneInteractionReveal, revealPaneStrips } from "./pane-strip-registry";
import { createPaneTabMemory, describePaneTab, prunePaneTabMemory, type PaneTabInfo } from "./pane-tab-model";
import { persistTabInfo } from "./pane-tab-persist";
import type { PaneTabColors } from "./PaneTabStrip";
import {
    emitPaneLocatePlay,
    LOCATE_READY_TIMEOUT_HIDDEN_MS,
    LOCATE_READY_TIMEOUT_MS,
    locateSlotWaitMs,
    onPaneLocate,
    recordLocateStart,
    samplePane,
    showLocateFlash,
    whenPaneReady,
    type ReadyGate,
} from "@/app/util/pane-locate";

const blockMeta = (blockId: string): Block["meta"] | undefined => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta;

function sameIds(a: string[], b: string[]): boolean {
    return a.length === b.length && a.every((id, i) => id === b[i]);
}

/** Whether `vm` is `blockId`'s own ViewModel. A ViewModel that doesn't
 *  expose its blockId is trusted (it can't be checked). */
function viewModelIsFor(vm: ViewModel | null, blockId: string): boolean {
    const owner = (vm as { blockId?: unknown } | null)?.blockId;
    return typeof owner !== "string" || owner === blockId;
}

/**
 * The pane's strips (pane tabs, and document tabs in an Editor or Media view) scroll their selected
 * tab into view when the pane takes focus and on any click or input in it
 * (SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md §3-§4).
 */
export function usePaneStripReveal(isFocused: Accessor<boolean>, getRoot: () => HTMLElement | undefined): void {
    onMount(() => {
        const root = getRoot();
        if (root) onCleanup(installPaneInteractionReveal(root));
    });
    createEffect(
        on(
            isFocused,
            (focused, wasFocused) => {
                const root = getRoot();
                if (focused && !wasFocused && root) revealPaneStrips(root, "focus");
            },
            { defer: true }
        )
    );
}

/**
 * The locate flash (pane-locate.ts): a reveal of one of this pane's blocks flashes the pane once it
 * is on screen. Any other reveal ends the one running or waiting here, so only the newest pane
 * flashes, and leaving the pane ends its own.
 */
export function usePaneLocateFlash(
    nodeModel: NodeModel,
    isFocused: Accessor<boolean>,
    activeBlockId: Accessor<string>,
    stackIds: Accessor<string[]>,
    getRoot: () => HTMLElement | undefined
): void {
    let locateGate: ReadyGate | null = null;
    let locateDelay: ReturnType<typeof setTimeout> | undefined;
    let stopLocateFlash: (() => void) | null = null;
    const endLocate = () => {
        locateGate?.cancel();
        locateGate = null;
        clearTimeout(locateDelay);
        stopLocateFlash?.();
        stopLocateFlash = null;
    };
    onMount(() => {
        const off = onPaneLocate((blockId) => {
            endLocate();
            if (!untrack(stackIds).includes(blockId)) return;
            const gate = whenPaneReady(() => samplePane(getRoot(), isFocused() && activeBlockId() === blockId), {
                timeoutMs: document.visibilityState === "visible" ? LOCATE_READY_TIMEOUT_MS : LOCATE_READY_TIMEOUT_HIDDEN_MS,
            });
            locateGate = gate;
            void gate.ready.then((ready) => {
                if (!ready || locateGate !== gate) return;
                const begin = () => {
                    const root = getRoot();
                    // Still the same target: the user may have switched tab while it waited.
                    if (locateGate !== gate || !root || !isFocused() || activeBlockId() !== blockId) return;
                    recordLocateStart(performance.now());
                    stopLocateFlash = showLocateFlash(root, {
                        light: isLightThemeActive(),
                        reduced: shouldReduceMotion("feedback"),
                    });
                    emitPaneLocatePlay(blockId, nodeModel.nodeId);
                };
                // Flashes start at least a gap apart. The newest request waits for
                // its turn rather than being dropped; a newer one replaces it.
                const wait = locateSlotWaitMs(performance.now());
                if (wait <= 0) begin();
                else locateDelay = setTimeout(begin, wait);
            });
        });
        onCleanup(() => {
            off();
            endLocate();
        });
    });
    // Leaving the pane ends its flash: it must not mark a pane the user has left.
    createEffect(
        on(
            isFocused,
            (focused) => {
                if (!focused) endLocate();
            },
            { defer: true }
        )
    );
}

/** The pane's tab ids: the layout's stack, in order. */
export function createStackIds(nodeModel: NodeModel): Accessor<string[]> {
    return createMemo<string[]>(() => [...paneStackIds(nodeModel)], undefined, { equals: sameIds });
}

/**
 * What the ACTIVE tab's view type contributes to this chrome (its manifest's `chrome`, Pane Tab
 * contract Phase 4). Each view type's model is built ONCE per pane (it creates signals of its own),
 * in the chrome's own reactive scope, the first time one of its tabs is active; switching tabs only
 * changes which model is read. A view type without `chrome` gets every default.
 */
export function createPaneChromeModel(
    nodeModel: NodeModel,
    activeBlockId: Accessor<string>,
    activeBlockData: Accessor<Block | undefined>
): Accessor<PaneChromeModel | null> {
    const chromeOwner = getOwner();
    const chromeModels = new Map<string, PaneChromeModel | null>();
    const activeView = createMemo(() => resolvePaneTabView((activeBlockData()?.meta?.view as string | undefined) ?? ""));
    return createMemo<PaneChromeModel | null>(() => {
        const view = activeView();
        if (!chromeModels.has(view)) {
            const build = getPaneTab(view)?.chrome;
            const anchor = untrack(activeBlockId);
            chromeModels.set(view, build ? (runWithOwner(chromeOwner, () => untrack(() => build(anchor, nodeModel))) ?? null) : null);
        }
        return chromeModels.get(view) ?? null;
    });
}

/** The stack's tab ids, then the chrome model's extra tabs (forks of other panes) that the stack doesn't hold. */
export function createTabIds(stackIds: Accessor<string[]>, extraTabs: Accessor<{ blockId: string }[]>): Accessor<string[]> {
    return createMemo<string[]>(
        () => {
            const stack = stackIds();
            const inStack = new Set(stack);
            return [...stack, ...extraTabs().map((t) => t.blockId).filter((id) => !inStack.has(id))];
        },
        undefined,
        { equals: sameIds }
    );
}

/**
 * Each tab described by the shared tab model, keyed by blockId, and the last real name and favicon
 * of every stack tab written to its meta, so no tab changes when it is first clicked.
 */
export function createTabInfos(
    nodeModel: NodeModel,
    stackIds: Accessor<string[]>,
    tabIds: Accessor<string[]>,
    extraTabs: Accessor<{ blockId: string; label?: string }[]>,
    activeBlockId: Accessor<string>
): Accessor<Map<string, PaneTabInfo>> {
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
            const meta = blockMeta(blockId);
            const view = meta?.view as string | undefined;
            let ordinal = 0;
            if (inStack.has(blockId)) {
                ordinal = (ordinals.get(view ?? "") ?? 0) + 1;
                ordinals.set(view ?? "", ordinal);
            }
            infos.set(
                blockId,
                describePaneTab(
                    {
                        blockId,
                        view,
                        meta,
                        ordinal,
                        // Only this block's OWN ViewModel may name it: a live name/favicon read
                        // from another tab's ViewModel would be remembered under this tab.
                        liveViewModel: blockId === activeId && viewModelIsFor(liveVm, blockId) ? liveVm : null,
                    },
                    extraLabels.get(blockId),
                    tabMemory
                )
            );
        }
        prunePaneTabMemory(tabMemory, infos.keys());
        return infos;
    });
    createEffect(() => {
        const inStack = new Set(stackIds());
        for (const info of tabInfos().values()) if (inStack.has(info.blockId)) persistTabInfo(info);
    });
    return tabInfos;
}

/**
 * Each pill's own pane color, from its own block's meta (the same source BlockFrame_Header's header
 * and border use). One call per pill's own block: a color taken from anything tab-wide would
 * collapse every underline to one shared color.
 */
export function createTabColors(tabIds: Accessor<string[]>): Accessor<Map<string, PaneTabColors>> {
    return createMemo(() => {
        const isLightTheme = isLightThemeActive();
        const colors = new Map<string, PaneTabColors>();
        for (const blockId of tabIds()) {
            const meta = blockMeta(blockId);
            colors.set(blockId, {
                // The identity normalised for the theme, so it clears 3:1 against the pill under it;
                // an uncoloured tab falls back to accent.
                underline: computeBlockIdentityColor(meta, isLightTheme),
                background: computeBlockTabPillBg(meta, isLightTheme),
                activeBackground: computeBlockTabPillActiveBg(meta, isLightTheme),
                // Always set: the header behind the strip is tinted with the ACTIVE block's color,
                // so a transparent uncolored pill would appear to take on whichever tab is selected.
                neutralBackground: computeBlockTabPillNeutralBg(meta, isLightTheme),
            });
        }
        return colors;
    });
}

/**
 * The header row's background when the pane's tabs don't agree on a single color
 * (SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md). The row is ONE element painted with the ACTIVE
 * block's color, so everything the pills don't cover (the "+", the gap after the last pill, the bar
 * out to the end icons) would change color on every tab switch. A pane of one identity is about that
 * color; one with several has no single color to be about, so it gets a neutral one. `undefined`
 * hands the header back to BlockFrame_Header's own color.
 */
export function createHeaderTailBg(tabIds: Accessor<string[]>): Accessor<string | undefined> {
    return createMemo(() => {
        const isLightTheme = isLightThemeActive();
        const ids = tabIds();
        // An uncoloured block's EFFECTIVE header background depends on its view (a non-agent block
        // gets a fixed default, an agent block stays translucent), so the key includes the view.
        // Not `computeBlockTabPillNeutralBg`: that collapses agent and non-agent to one value in
        // light theme, while the header fallback does not.
        const headerKeyOf = (meta: Block["meta"] | undefined): string =>
            computeBlockColorBg(meta, isLightTheme) ??
            (paneTabCapability(meta?.view, "header") === "surface" ? "\u0000surface-default" : "\u0000fixed-default");
        // Count IDENTITIES, not tabs: an uncoloured tab (Accounts, CPU, a terminal) is not a competing
        // identity, so a pane with one coloured agent plus utility tabs keeps that agent's tint
        // whichever tab is selected. Only a pick or an agent colour is an identity; a tab coloured
        // just by its widget type counts as uncoloured (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.8).
        const identities = new Set<string>();
        let anyUncoloured = false;
        for (const blockId of ids) {
            const bg = computeBlockColorBg(blockMeta(blockId), isLightTheme, { widget: false });
            if (bg) identities.add(bg);
            else anyUncoloured = true;
        }
        // Every tab shares the one identity: BlockFrame_Header already paints it for the active tab.
        if (identities.size === 1 && !anyUncoloured) return undefined;
        if (identities.size === 1) return [...identities][0];
        if (identities.size > 1) return computeMixedPaneHeaderBg(isLightTheme);
        // No identity at all: tabs that agree on the resolved header key leave the header alone.
        const distinct = new Set<string>();
        for (const blockId of ids) {
            distinct.add(headerKeyOf(blockMeta(blockId)));
            if (distinct.size > 1) break;
        }
        if (distinct.size <= 1) return undefined;
        // One value per theme, independent of every block's meta.
        return computeMixedPaneHeaderBg(isLightTheme);
    });
}

/** What a pill's tooltip adds for an agent tab: the summary line the Swarm row shows, or the browser identity its icon marks. */
export function tabSummaryOf(id: string, tabInfos: Accessor<Map<string, PaneTabInfo>>): string | undefined {
    const badge = tabInfos().get(id)?.badge;
    if (badge) return `Browsing as ${badge.label}`;
    const meta = blockMeta(id);
    if (meta?.view !== "agent") return undefined;
    return readSwarmSummary(meta) ?? undefined;
}

/** An agent tab's seat, from the `agent:seat` srv seeds at agent.open (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4). */
export function tabSeatOf(id: string): string | undefined {
    const meta = blockMeta(id);
    if (meta?.view !== "agent") return undefined;
    const seat = readSeatMeta(meta);
    return seat ? seatTooltipLine(seat) : undefined;
}

/**
 * Rename (double-click a pill). The override shows the new name at once, before the write round-trips
 * back through the block's meta, and is rolled back with a notice if the write fails.
 */
export function createTabRename(tabInfos: Accessor<Map<string, PaneTabInfo>>) {
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
    return { renamingId, setRenamingId, labelOf, confirmRename };
}

/**
 * The header menu for a pill in a pane of several tabs, built for that tab. A pill that is not in
 * front has just been brought there, and the header's handler still belongs to the tab that was, so
 * the menu is shown once the new tab's view model is in place. The pane's tabs as a whole add
 * "Merge document tabs", for two document panes in one pane that no move merged (a saved layout,
 * an agent's move).
 */
function openPaneTabMenu(e: MouseEvent, layoutModel: NodeModel["layoutModel"], nodeModel: NodeModel, blockId: string, inFront: boolean): void {
    const extra: ContextMenuItem[] = [];
    if (mergeableDocumentPanes(layoutModel, nodeModel.nodeId).length >= 2) {
        extra.push({ label: "Merge document tabs", icon: "object-group", click: () => void mergeDocumentPanesInStack(layoutModel, nodeModel.nodeId) });
    }
    const show = () => {
        const vm = nodeModel.activeViewModel?.() ?? null;
        showPaneTabMenu(e, nodeModel, blockId, viewModelIsFor(vm, blockId) ? vm : null, extra);
    };
    if (inFront) show();
    else setTimeout(show, 0);
}

/** The pill handlers: select, activate, close, context menu, add and same-pane reorder. */
export function createPaneTabHandlers(
    nodeModel: NodeModel,
    model: Accessor<PaneChromeModel | null>,
    stackIds: Accessor<string[]>,
    activeBlockId: Accessor<string>
) {
    const layoutModel = nodeModel.layoutModel;
    // A click on a pill of a collapsed pane expands it, the active pill included (handleActivate
    // never sees that click) and an agent pane's own onActivate included (it returns before the
    // ordinary path). Restore first, then the activation, the order showBlockInPane uses
    // (SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §6). An extra tab belongs to
    // another pane, and its click reveals that pane: this pane is not expanded for it.
    const handleSelect = (blockId: string) => {
        if (nodeModel.isMinimized?.() && stackIds().includes(blockId)) layoutModel.minimizeNodeToggle(nodeModel.nodeId);
    };
    const handleActivate = (blockId: string) => {
        // A view type that handles activation itself (its own tab model) returns true; anything else
        // falls through to the same-pane switch.
        if (model()?.onActivate?.(blockId) === true) return;
        if (blockId === activeBlockId()) return;
        setActiveBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    };
    const handleClose = (blockId: string) => {
        if (model()?.onClose?.(blockId) === true) return;
        void closeBlockInStack(layoutModel, nodeModel.nodeId, blockId);
    };
    // A pill's menu is the header menu for that tab (openPaneTabMenu). A single-tab pane's pill, and an
    // extra tab of another pane (its Close Pane and magnify are not this pane's), leave the event to
    // the header's own handler.
    const handleTabContextMenu = (blockId: string, e: MouseEvent) => {
        if (stackIds().length < 2 || !stackIds().includes(blockId)) return;
        e.preventDefault();
        e.stopPropagation();
        const inFront = blockId === activeBlockId();
        if (!inFront) handleActivate(blockId);
        openPaneTabMenu(e, layoutModel, nodeModel, blockId, inFront);
    };
    const handleAdd = (e?: MouseEvent) => {
        if (!e) return;
        openPaneTabWidgetPicker(layoutModel, nodeModel.nodeId, e, model()?.newTabMeta);
    };
    // Same-pane drag-reorder (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.2). No `model()?.onReorder`
    // hook: reordering changes no membership and needs no view-type side effect.
    const handleReorder = (blockId: string, targetId: string, position: "before" | "after") =>
        moveBlockInStack(layoutModel, blockId, targetId, position);
    return { handleSelect, handleActivate, handleClose, handleTabContextMenu, handleAdd, handleReorder };
}

/**
 * What the pane does with tabs dropped on it (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4): onto
 * its header (appended, active, at the end of THIS pane's stack, from whichever pane the pill came
 * from), several at once, and onto its body (a pane of its own on that side).
 */
export function createPaneTabDrops(nodeModel: NodeModel, stackIds: Accessor<string[]>, activeBlockId: Accessor<string>) {
    const layoutModel = nodeModel.layoutModel;
    const foreign = foreignTabMoves(layoutModel, activeBlockId); // a pill from another window tab goes through the one cross-tab move
    // `target` is any current member of THIS pane (its active tab), which is all `moveBlockInStack`
    // needs to resolve the destination; the dragged block's own pane is resolved from `blockId`.
    const handleReceiveForeignTab = (blockId: string, opts?: { keepSeparate?: boolean }) => {
        const target = activeBlockId();
        if (!target) return false;
        if (foreign.isForeign(blockId)) return foreign.move([blockId], blockId, { asTab: true });
        const moved = opts ? moveBlockInStack(layoutModel, blockId, target, "end", true, opts) : moveBlockInStack(layoutModel, blockId, target, "end", true);
        // A lone picked tab that was dragged and dropped is done with: it is no longer picked.
        if (moved && paneTabSelection().has(blockId)) clearPaneTabSelection();
        // A collapsed pane stays collapsed: only a plain click on a pill expands a pane
        // (pane-tab-gesture.ts), never a drop.
        return moved;
    };
    // Several tabs (a Ctrl/Cmd+click selection) dropped on this pane, in one move. The anchor is
    // this pane's visible tab, which is never one of the moved tabs (blocks already here are left
    // alone). SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.3.
    const handleReceiveForeignTabs = (blockIds: readonly string[], grabbed: string, opts?: { keepSeparate?: boolean }) => {
        const target = activeBlockId();
        if (!target) return false;
        if (blockIds.length > 0 && foreign.isForeign(blockIds[0])) return foreign.move(blockIds, grabbed, { asTab: true });
        const moved = opts ? moveBlocksInStack(layoutModel, blockIds, target, grabbed, opts) : moveBlocksInStack(layoutModel, blockIds, target, grabbed);
        if (moved === false) return false;
        clearPaneTabSelection();
        return moved;
    };
    // A single pill dropped on this pane's body, outside its middle, becomes a pane of its own on that
    // side; the new pane's key is where the pill bounces.
    // SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md.
    const bodyDrop = {
        model: layoutModel,
        // No body zones while a pane is maximized: the layout keeps every pane's tiled rectangle, which is
        // not where the displayed pane is, so the pointer could not be placed against it.
        hasBody: () => !(nodeModel.isMinimized?.() ?? false) && !layoutModel.treeState.magnifiedNodeId,
        place: (blockId: string, zone: Extract<PillZone, { kind: "place" }>): string | false => {
            if (foreign.isForeign(blockId)) return foreign.move([blockId], blockId, { direction: clampCrossTabDirection(zone.direction)! });
            const landedIn = placePill(layoutModel, blockId, nodeModel.nodeId, zone);
            if (landedIn !== false && paneTabSelection().has(blockId)) clearPaneTabSelection();
            return landedIn;
        },
    };
    const acceptsForeignTabs = (blockIds: readonly string[]) => {
        const here = new Set(stackIds());
        return blockIds.some((id) => !here.has(id));
    };
    return { handleReceiveForeignTab, handleReceiveForeignTabs, bodyDrop, acceptsForeignTabs };
}

/** The selection belongs to the Window Tab on screen: a tab that was closed, or that lives in another Window Tab's layout, is dropped from it. */
export function usePaneTabSelectionPruning(layoutModel: NodeModel["layoutModel"]): void {
    const windowTabDisplayed = useWindowTabDisplayed();
    createEffect(() => {
        layoutModel.localTreeStateAtom();
        if (!windowTabDisplayed()) return;
        prunePaneTabSelection(allBlockIds(layoutModel.treeState.rootNode));
    });
}

/**
 * The busy-indicator slot is the chrome's own, on every pane, not a `model` capability: `model`
 * belongs to whichever view was active when this pane first hoisted, so an agent added as a tab to a
 * pane that started as Swarm or a terminal got no slot. The handoff follows the ACTIVE view model,
 * re-pointing on every switch; a view model without `setProgressBarMount` leaves the slot empty.
 */
export function useProgressBarSlot(nodeModel: NodeModel) {
    const [progressSlotEl, setProgressSlotEl] = createSignal<HTMLDivElement | null>(null);
    createEffect(() => {
        const vm = nodeModel.activeViewModel?.() ?? null;
        const el = progressSlotEl();
        vm?.setProgressBarMount?.(el);
        onCleanup(() => vm?.setProgressBarMount?.(null));
    });
    return setProgressSlotEl;
}
