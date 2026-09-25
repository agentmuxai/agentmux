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
import {
    computeBlockActiveBorderColor,
    computeBlockColorBg,
    computeBlockTabPillBg,
    computeBlockTabPillNeutralBg,
    computeFocusRingBorderColor,
    computeMixedPaneHeaderBg,
} from "@/app/block/blockframe";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";
import { getSettingsKeyAtom, MOS, pushNotification } from "@/app/store/global";
import { ErrorBoundary } from "@/element/errorboundary";
import { closeBlockInStack, moveBlockInStack, setActiveBlockInStack, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import "./PaneChrome.scss";
import { openPaneTabWidgetPicker } from "./pane-tab-picker";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";
import { createPaneTabMemory, describePaneTab, PaneTabIconView, prunePaneTabMemory, type PaneTabInfo } from "./pane-tab-model";
import { PaneTabRenameInput } from "./PaneTabRenameInput";
import type { PaneTabColors } from "./PaneTabStrip";

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
        computeFocusRingBorderColor(isFocused(), activeBlockData()?.meta)
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

    // Each pill's own pane color (SPEC_AGENT_COLOR_2026_08_08.md's identity
    // color / the "Pane Color" picker's frame:hue), same source
    // BlockFrame_Header's own header/border use — every stack member is a
    // distinct blockId with its own meta, so a fork with its own color must
    // show it as a pill even while another fork is active. Read here (not
    // folded into tabInfos above) since it needs the current theme's
    // polarity, which label/icon description has no reason to depend on.
    //
    // computeBlockActiveBorderColor, one call per pill's own block: a pill's
    // color must never come from anything tab-wide, or every pill's
    // underline collapses to one shared color (reagent P1, PR #3484 — back
    // when a tab-level override tier still existed).
    const tabColors = createMemo(() => {
        const themeId = getSettingsKeyAtom("window:theme")();
        const isLightTheme = typeof themeId === "string" && LIGHT_THEME_IDS.has(themeId);
        const colors = new Map<string, PaneTabColors>();
        for (const blockId of tabIds()) {
            const meta = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta;
            const underline = computeBlockActiveBorderColor(meta);
            const background = computeBlockTabPillBg(meta, isLightTheme);
            // Always set: the header behind the strip is tinted with the
            // ACTIVE block's color, so a transparent uncolored pill would
            // appear to take on whichever tab is selected.
            const neutralBackground = computeBlockTabPillNeutralBg(meta, isLightTheme);
            colors.set(blockId, { underline, background, neutralBackground });
        }
        return colors;
    });

    // The header row's background, when the pane's tabs don't agree on a
    // single color — SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md.
    //
    // The row is ONE element, painted by BlockFrame_Header with the ACTIVE
    // block's color, and the tab strip sits on top of it. Everything the
    // pills don't cover (the "+", the gap after the last pill, the run of
    // bar out to the end icons — the "tail") is therefore the active tab's
    // color, and changes on every tab switch. Fine when a pane held one
    // widget or a stack of same-colored forks; the universal pane-tabs
    // redesign made a pane of differently-colored widgets the normal case,
    // where the tail became a large block of color that moved.
    //
    // A pane with one color is *about* that color. A pane with several has
    // no single color to be about, so picking one isn't information.
    //
    // Computed here rather than in blockframe.tsx because this is the only
    // component that has both the pane's whole tab list and the theme;
    // BlockFrame_Header renders one block and is handed a color, which
    // keeps every tab-set-aware decision in one place and leaves every
    // non-PaneChrome BlockFrame consumer untouched by construction.
    const headerTailBg = createMemo(() => {
        const themeId = getSettingsKeyAtom("window:theme")();
        const isLightTheme = typeof themeId === "string" && LIGHT_THEME_IDS.has(themeId);
        const ids = tabIds();
        // Compare the RESOLVED background strings, not the meta that
        // produced them: two blocks that land on the same rendered color
        // are the same color here even if one got there via frame:hue and
        // the other via its agent identity color.
        //
        // `undefined` (no color of its own) is NOT one member — uncolored
        // blocks do not all render the same header.
        //
        // `BlockFrame_Header`'s own fallback (blockframe.tsx) branches on
        // `meta.view` when there is no color: a non-agent block gets
        // `NON_AGENT_DEFAULT_HEADER_BG`, an agent block gets nothing and
        // stays translucent. So an uncolored block's EFFECTIVE header
        // background is a function of its view, and keying this set on the
        // color alone made a pane of two uncolored tabs — one agent, one
        // not — collapse to `size === 1`, skip the override entirely, and
        // hand the header straight back to that per-view fallback. The tail
        // then changed color with the active tab: the exact bug this
        // component exists to remove, surviving in the one case nothing
        // tested (reagent P1 on #3492).
        //
        // Deliberately not `computeBlockTabPillNeutralBg` as the key: that
        // helper is theme-conditional (it collapses agent and non-agent to
        // one value in light theme) while the header fallback above is not,
        // so it would under-discriminate on a light-theme mixed pane.
        const headerKeyOf = (meta: Block["meta"] | undefined): string =>
            computeBlockColorBg(meta, isLightTheme) ??
            (meta?.view === "agent" ? "\u0000agent-default" : "\u0000non-agent-default");
        const distinct = new Set<string>();
        for (const blockId of ids) {
            const meta = MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta;
            distinct.add(headerKeyOf(meta));
            if (distinct.size > 1) break;
        }
        // The tabs agree — hand back NOTHING and let BlockFrame_Header do
        // exactly what it always did (the active block's own color, else
        // its agent/non-agent default). That covers both "they all share a
        // color" (the active block resolves to that same color anyway) and
        // "none of them has one", so an uncolored single-agent pane keeps
        // its untouched default instead of being forced to a value this
        // memo picked. Only a genuinely mixed pane needs an override.
        if (distinct.size <= 1) return undefined;
        // One value per theme, independent of every block's meta — see
        // computeMixedPaneHeaderBg for why the pill's own neutral (which
        // varies by meta.view) is the wrong thing here (reagent P1).
        return computeMixedPaneHeaderBg(isLightTheme);
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
    // Same-pane drag-reorder (Phase 3, SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.2). No `model?.onReorder` escape hatch like activate/close have —
    // unlike activation (agent forks can live in a DIFFERENT pane) or close
    // (a view type may need its own confirmation/cleanup), reordering never
    // changes membership or requires side effects beyond the stack itself,
    // so there's nothing for a view type to meaningfully override.
    const handleReorder = (blockId: string, targetId: string, position: "before" | "after") =>
        moveBlockInStack(layoutModel, blockId, targetId, position);
    // Cross-pane drop-to-append (Phase 4, SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.4): a pill dragged from a DIFFERENT pane was dropped on this
    // Pane's header — append it, active, at the end of THIS pane's stack.
    // `target` is any current member of THIS pane (its active tab), which is
    // all `moveBlockInStack` needs to resolve the destination; the dragged
    // block's own pane is resolved from `blockId`, NOT assumed to be this
    // one (ReAgent P0 on PR #3447 — passing this pane's node id here is
    // exactly what made the first version a silent no-op).
    const handleReceiveForeignTab = (blockId: string) => {
        const target = activeBlockId();
        if (!target) return false;
        return moveBlockInStack(layoutModel, blockId, target, "end", true);
    };

    const activeViewModelOrUndefined = () => nodeModel.activeViewModel?.() ?? undefined;

    const renderHeader = (viewModel: ViewModel | null): JSX.Element => (
        <PaneHeaderTabStrip
            tabs={tabIds()}
            activeId={activeBlockId()}
            getId={(id) => id}
            getLabel={labelOf}
            getIcon={(id) => <PaneTabIconView icon={() => tabInfos().get(id)?.icon} />}
            getColor={(id) => tabColors().get(id)}
            headerBgOverride={headerTailBg()}
            onActivate={handleActivate}
            onClose={handleClose}
            onReorder={handleReorder}
            onReceiveForeignTab={handleReceiveForeignTab}
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
            // The whole-pane drop zone for a Pane Tab dragged from another
            // pane (PaneTabStrip's `foreignDropRootFor`).
            data-role="pane"
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
