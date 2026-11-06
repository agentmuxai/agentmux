// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";
import { FilterInput, Kbd } from "@/app/element/ui";
import { atoms, MOS } from "@/app/store/global";
import { replaceBlock } from "@/app/store/block-layout-actions";
import { closeBlockInStack, getLayoutModelForStaticTab } from "@/layout/index";
import { findOpenBlockByView } from "@/app/util/reveal-block";
import { isSingletonView } from "@/app/store/widget-singleton";
import {
    getChildWidgets,
    getHiddenKeys,
    getPinnedKeys,
    isWidgetHidden,
    openWidget,
    widgetEntryColor,
} from "@/app/window/action-widgets-config";
import { checkKeyPressed, keydownWrapper } from "@/util/keyutil";
import { isBlank, makeIconClass, createSignalAtom } from "@/util/util";
import type { SignalAtom } from "@/util/util";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { PaneTabManifest } from "@/app/store/pane-tab-registry";
import { blockDefFromSource, SHORTCUTS_FROM_META } from "@/app/store/shortcuts-from";
import { orderByRecent } from "@/app/store/shortcuts-recent";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { tip } from "@/app/element/tip";
import { readZoom } from "@/app/store/zoom-factor";
import "./shortcuts.scss";

function sortByDisplayOrder(
    wmap: { [key: string]: WidgetConfigType } | null | undefined
): [string, WidgetConfigType][] {
    if (!wmap) return [];
    const entries = Object.entries(wmap);
    entries.sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));
    return entries;
}

/** Whether a pane of `view` is open in this window, mounted or not (an inactive tab of a pane's stack is not). */
const isViewOpen = async (view: string): Promise<boolean> => (await findOpenBlockByView(view)) != null;

/** What the arrow keys need of the grid: how many tiles sit in a row, read from the rendered grid. */
type GridLayoutType = { columns: number };

/** The Shortcuts pane's state behind its native pane tab (`shortcutsPaneTab`). */
/** Exported for the tests; the app reaches it through `shortcutsPaneTab`. */
export class ShortcutsViewModel {
    blockId: string;
    searchTerm: SignalAtom<string>;
    selectedIndex: SignalAtom<number>;
    /**
     * A parent widget (e.g. "Messengers") the grid has drilled into — its
     * children replace the top-level grid until Escape/Back pops back out.
     * SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md doesn't cover this
     * surface directly, but a parent has no blockdef of its own to launch,
     * so clicking/Entering one has to do *something* other than try to open
     * an empty pane — drill-down mirrors how every other picker in the app
     * now reaches a group's children (nested submenu / flyout).
     */
    activeParent: SignalAtom<WidgetConfigType | null>;
    gridLayout: GridLayoutType = null;
    inputRef: { current: HTMLInputElement | null } = { current: null };

    constructor(blockId: string) {
        this.blockId = blockId;
        this.searchTerm = createSignalAtom("");
        this.selectedIndex = createSignalAtom(0);
        this.activeParent = createSignalAtom<WidgetConfigType | null>(null);
    }

    filteredWidgets(): WidgetConfigType[] {
        const searchTerm = this.searchTerm();
        const fullConfig = atoms.fullConfigAtom();
        const wmap = fullConfig?.widgets || {};
        const settings = fullConfig?.settings || {};
        // A group that's been hidden since the user drilled into it (Settings → Widgets, the bar)
        // drops the Shortcuts pane back to the top-level grid, where it no longer shows either.
        const drilled = this.activeParent();
        const drilledKey = drilled
            ? Object.keys(wmap).find((k) => wmap[k] === drilled || (wmap[k]?.label === drilled.label && wmap[k]?.children?.join() === drilled.children?.join()))
            : undefined;
        const parent = drilled && !(drilledKey && isWidgetHidden(drilledKey, wmap, settings)) ? drilled : null;

        let source: WidgetConfigType[];
        if (parent) {
            // Drilled into a group — a child that's since been individually
            // pinned (promoted out via its own "Pin to bar") is excluded
            // here; it now shows at the top level instead, not doubled up.
            // The last used of the group's tiles come first here too.
            const children = getChildWidgets(parent, wmap, settings).map((c) => [c.key, c.widget] as const);
            source = orderByRecent(children, settings["launcher:recent"]).map(([, widget]) => widget);
        } else {
            // display:grouponly normally keeps a grouped child out of this
            // top-level grid — but an individually pinned child (promoted
            // out of its group) overrides that, same as it overrides the
            // group-hiding default everywhere else (see
            // getEffectiveGroupedChildKeys).
            const pinnedSet = new Set(getPinnedKeys(settings, wmap));
            const hidden = getHiddenKeys(settings);
            const shown = sortByDisplayOrder(wmap)
                .filter(([key, widget]) => !widget["display:grouponly"] || pinnedSet.has(key.replace("defwidget@", "")))
                // Hidden by the user in Settings → Widgets, like the bar and the "+" picker.
                .filter(([key]) => !isWidgetHidden(key, wmap, settings, hidden));
            // The pane types the person opened last come first, the rest keep their configured order.
            source = orderByRecent(shown, settings["launcher:recent"]).map(([, widget]) => widget);
        }

        return source.filter((widget) => matchesEveryWord(searchTerm, widget.label));
    }

    /** True when a tile click/Enter should drill down instead of opening a pane. */
    isParentWidget(widget: WidgetConfigType): boolean {
        return (widget.children?.length ?? 0) > 0;
    }

    drillInto(widget: WidgetConfigType): void {
        this.activeParent._set(widget);
        this.searchTerm._set("");
        this.selectedIndex._set(0);
    }

    goBack(): void {
        this.activeParent._set(null);
        this.searchTerm._set("");
        this.selectedIndex._set(0);
    }

    giveFocus(): boolean {
        if (this.inputRef.current) {
            this.inputRef.current.focus();
            return true;
        }
        return false;
    }

    keyDownHandler(e: MuxKeyboardEvent): boolean {
        if (this.gridLayout == null) return false;
        const gridLayout = this.gridLayout;
        const filteredWidgets = this.filteredWidgets();
        const selectedIndex = this.selectedIndex();
        const rows = Math.ceil(filteredWidgets.length / gridLayout.columns);
        const currentRow = Math.floor(selectedIndex / gridLayout.columns);
        const currentCol = selectedIndex % gridLayout.columns;

        if (checkKeyPressed(e, "ArrowUp")) {
            if (filteredWidgets.length == 0) return true;
            if (currentRow > 0) {
                const newIndex = selectedIndex - gridLayout.columns;
                if (newIndex >= 0) this.selectedIndex._set(newIndex);
            }
            return true;
        }
        if (checkKeyPressed(e, "ArrowDown")) {
            if (filteredWidgets.length == 0) return true;
            if (currentRow < rows - 1) {
                const newIndex = selectedIndex + gridLayout.columns;
                if (newIndex < filteredWidgets.length) this.selectedIndex._set(newIndex);
            }
            return true;
        }
        if (checkKeyPressed(e, "ArrowLeft")) {
            if (filteredWidgets.length == 0) return true;
            if (currentCol > 0) this.selectedIndex._set(selectedIndex - 1);
            return true;
        }
        if (checkKeyPressed(e, "ArrowRight")) {
            if (filteredWidgets.length == 0) return true;
            if (currentCol < gridLayout.columns - 1 && selectedIndex + 1 < filteredWidgets.length) {
                this.selectedIndex._set(selectedIndex + 1);
            }
            return true;
        }
        // Shift+Enter opens a new pane for a singleton instead of bringing the open one back, as a Shift-click does.
        if (checkKeyPressed(e, "Enter") || checkKeyPressed(e, "Shift:Enter")) {
            if (filteredWidgets.length == 0) return true;
            if (filteredWidgets[selectedIndex]) {
                void this.handleWidgetSelect(filteredWidgets[selectedIndex], { forceNew: e.shift === true });
            }
            return true;
        }
        if (checkKeyPressed(e, "Escape")) {
            // Stages: clear an in-progress search first; then pop back out of a
            // drilled-into group, so "Escape, Escape" from a filtered child list
            // doesn't skip straight past the group's own child list; then, for a pane
            // a split opened, close it (a question the person can dismiss).
            if (this.searchTerm() === "" && this.activeParent() !== null) {
                this.goBack();
            } else if (this.searchTerm() === "" && this.openedBySplit()) {
                void this.closeSelf();
            } else {
                this.searchTerm._set("");
                this.selectedIndex._set(0);
            }
            return true;
        }
        return false;
    }

    /** The pane's own meta (the split it came from is in `shortcuts:from`). */
    private ownMeta(): Record<string, unknown> | undefined {
        return MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", this.blockId))()?.meta;
    }

    /** True for a pane a split opened; one opened by `app:defaultnewblock` has no source. */
    openedBySplit(): boolean {
        return typeof this.ownMeta()?.[SHORTCUTS_FROM_META] === "string";
    }

    /** The meta of the pane that was split to open this one, while that pane is still there. */
    private sourceMeta(): Record<string, unknown> | undefined {
        const from = this.ownMeta()?.[SHORTCUTS_FROM_META];
        if (typeof from !== "string") return undefined;
        return MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", from))()?.meta;
    }

    /** Closes this tab. It has no content to lose, so nothing is asked and the neighbour takes its
     *  space. Only this block goes: when it was stacked with other panes the rest stay, and when it
     *  is the pane's last tab the pane closes. */
    async closeSelf(): Promise<void> {
        const layoutModel = getLayoutModelForStaticTab();
        const node = layoutModel.getNodeByBlockId(this.blockId);
        if (node) await closeBlockInStack(layoutModel, node.id, this.blockId);
    }

    /**
     * A tile for a singleton view (Memory, Connectors...) whose pane is already open brings that pane
     * back, as the widget bar does, and this pane closes: it was a question and has been answered.
     * `forceNew` (Shift) opens a new pane in this one's place instead, and so does a singleton that is not open.
     */
    async handleWidgetSelect(widget: WidgetConfigType, opts: { forceNew?: boolean } = {}) {
        if (this.isParentWidget(widget)) {
            this.drillInto(widget);
            return;
        }
        const view = widget.blockdef?.meta?.["view"] as string | undefined;
        if (view && !opts.forceNew && !widget.magnified && (widget.singleton ?? isSingletonView(view)) && (await isViewOpen(view))) {
            try {
                // This pane first: closing it afterwards would hand the focus to its neighbour.
                await this.closeSelf();
                await openWidget(widget);
            } catch (error) {
                console.error("Error bringing back the open pane:", error);
            }
            return;
        }
        try {
            // A Terminal or Files tile starts where the pane that was split was.
            await replaceBlock(this.blockId, blockDefFromSource(widget.blockdef, this.sourceMeta()), true);
        } catch (error) {
            console.error("Error replacing block:", error);
        }
    }
}

/** `zoom`: the pane's zoom (`term:zoom`), applied to the whole view, so the tiles, their icons and
 *  labels, the filter and the hints all scale with it; 1 when not given. */
function ShortcutsView(props: { model: ShortcutsViewModel; zoom?: () => number }): JSX.Element {
    const zoom = () => props.zoom?.() ?? 1;
    const model = props.model;
    const searchTerm = model.searchTerm;
    const selectedIndex = model.selectedIndex;
    const filteredWidgets = createMemo(() => model.filteredWidgets());

    let gridRef!: HTMLDivElement;

    // The tiles are a fixed size and wrap (`.shortcuts-grid`), so how many sit in a row is the
    // browser's to work out. The arrow keys read it back from the grid's resolved tracks.
    const measureColumns = () => {
        const tracks = getComputedStyle(gridRef).gridTemplateColumns.split(/\s+/).filter((t) => t.endsWith("px")).length;
        model.gridLayout = { columns: Math.max(1, tracks) };
    };

    onMount(() => {
        // The filter is the pane's keyboard: it has the focus as soon as the pane opens.
        queueMicrotask(() => model.giveFocus());
        measureColumns();
        const resizeObserver = new ResizeObserver(measureColumns);
        resizeObserver.observe(gridRef);
        return () => resizeObserver.disconnect();
    });

    // Reset selection when search term changes
    createEffect(() => {
        searchTerm(); // track
        selectedIndex._set(0);
    });

    // The grid scrolls, so the tile the arrows land on may be out of sight.
    createEffect(() => {
        const tile = gridRef?.children[selectedIndex()] as HTMLElement | undefined;
        tile?.scrollIntoView?.({ block: "nearest" });
    });

    // What Escape does now, for the hint line (the stages are in keyDownHandler).
    const escapeHint = () =>
        searchTerm() !== "" ? "clear the filter" : model.activeParent() ? "go back" : model.openedBySplit() ? "close" : null;

    // A column: the filter on top, the tiles scrolling in the middle, the hints below. Each has a
    // row of its own, so none can sit over another however small the pane is (shortcuts.scss).
    return (
        <div class="shortcuts-view" style={{ zoom: zoom() }}>
            {/* Faded logo behind the tiles, as in an empty Editor */}
            <EmptyPaneLogo />
            <div class="shortcuts-top">
                <div class="w-full max-w-[420px] flex">
                    <FilterInput
                        value={searchTerm()}
                        onInput={(q) => model.searchTerm._set(q)}
                        onKeyDown={keydownWrapper(model.keyDownHandler.bind(model))}
                        inputRef={(el) => (model.inputRef.current = el)}
                        placeholder="Filter shortcuts"
                        testId="shortcuts-filter"
                    />
                </div>
                {/* Breadcrumb — shown only once drilled into a parent widget
                    (e.g. "Messengers"). Escape does the same thing; this is the
                    mouse-driven equivalent for discoverability. */}
                <Show when={model.activeParent()}>
                    {(parent) => (
                        <div
                            class="mt-2 flex items-center gap-1 text-secondary text-xs cursor-pointer hover:text-foreground"
                            onClick={() => model.goBack()}
                        >
                            <i class="fa-solid fa-chevron-left" />
                            <span>{parent().label}</span>
                        </div>
                    )}
                </Show>
            </div>
            <div class="shortcuts-scroll">
                {/* Grid of widgets */}
                <div ref={gridRef!} class="shortcuts-grid" data-testid="shortcuts-grid">
                    <For each={filteredWidgets()}>
                        {(widget, index) => (
                            <div
                                onClick={(e) => void model.handleWidgetSelect(widget, { forceNew: e.shiftKey })}
                                {...tip(widget.description || widget.label)}
                                class={clsx(
                                    "shortcuts-tile rounded-md text-center",
                                    "transition-colors duration-150",
                                    index() === selectedIndex()
                                        ? "bg-highlightbg text-foreground"
                                        : "bg-hover hover:bg-highlightbg text-secondary hover:text-foreground"
                                )}
                            >
                                <div class="shortcuts-tile-icon relative" style={{ color: widgetEntryColor(widget, "identity") }}>
                                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })} />
                                    <Show when={model.isParentWidget(widget)}>
                                        <i class="fa-solid fa-chevron-right absolute -right-2.5 top-1/2 -translate-y-1/2 text-[8px] opacity-60" />
                                    </Show>
                                </div>
                                <Show when={!isBlank(widget.label)}>
                                    <div class="shortcuts-tile-label">{widget.label}</div>
                                </Show>
                            </div>
                        )}
                    </For>
                </div>
            </div>
            <div
                class="shortcuts-hints flex flex-wrap items-center justify-center gap-x-5 gap-y-1 text-sm text-foreground"
                data-testid="shortcuts-hints"
            >
                <Show
                    when={filteredWidgets().length > 0}
                    fallback={
                        <span>
                            Nothing matches "{searchTerm()}". Press <Kbd>Esc</Kbd> to clear the filter.
                        </span>
                    }
                >
                    <span class="inline-flex items-center gap-1">
                        <Kbd>↑</Kbd>
                        <Kbd>↓</Kbd>
                        <Kbd>←</Kbd>
                        <Kbd>→</Kbd>
                        <span class="ml-1">move</span>
                    </span>
                    <span class="inline-flex items-center gap-1.5">
                        <Kbd>Enter</Kbd> open
                    </span>
                    <Show when={escapeHint()}>
                        {(hint) => (
                            <span class="inline-flex items-center gap-1.5">
                                <Kbd>Esc</Kbd> {hint()}
                            </span>
                        )}
                    </Show>
                </Show>
            </div>
        </div>
    );
}

/** The Shortcuts pane as a native pane tab (Pane Tab contract Phase 2c): tiles for every pane
 *  type and widget, the last used first. It draws no header of its own: its search box is its
 *  top edge. `launcher` is its old name, still found in saved layouts and `app:defaultnewblock`. */
export const shortcutsPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "shortcuts",
    aliases: ["launcher"],
    label: "Shortcuts",
    icon: "shapes",
    defaultColor: "pink",
    // The shared per-pane zoom (Ctrl+scroll, Ctrl/Cmd +/-/0, the all-panes gesture); 12 px is its
    // text size. A zoom set here goes no further: a tile replaces this pane with a new one.
    capabilities: { header: "none", paneZoom: { baseFontSize: 12 } },
    create: (ctx) => {
        const model = new ShortcutsViewModel(ctx.blockId);
        return {
            component: () => <ShortcutsView model={model} zoom={() => readZoom(ctx.meta())} />,
            liveTitle: () => ({ text: "Shortcuts" }),
            focus: () => model.giveFocus(),
            onKeyDown: (e) => model.keyDownHandler(e),
        };
    },
};
