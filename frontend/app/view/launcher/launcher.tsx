// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import logoUrl from "@/app/asset/logo.svg?url";
import { atoms, replaceBlock } from "@/app/store/global";
import { checkKeyPressed, keydownWrapper } from "@/util/keyutil";
import { isBlank, makeIconClass, createSignalAtom } from "@/util/util";
import type { SignalAtom } from "@/util/util";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";

function sortByDisplayOrder(wmap: { [key: string]: WidgetConfigType } | null | undefined): WidgetConfigType[] {
    if (!wmap) return [];
    const wlist = Object.values(wmap);
    wlist.sort((a, b) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));
    return wlist;
}

type GridLayoutType = { columns: number; tileWidth: number; tileHeight: number; showLabel: boolean };

export class LauncherViewModel implements ViewModel {
    blockId: string;
    viewType = "launcher";
    viewIcon: SignalAtom<string>;
    viewName: SignalAtom<string>;
    viewComponent = LauncherView;
    noHeader: SignalAtom<boolean>;
    searchTerm: SignalAtom<string>;
    selectedIndex: SignalAtom<number>;
    containerSize: SignalAtom<{ width: number; height: number }>;
    gridLayout: GridLayoutType = null;
    inputRef: { current: HTMLInputElement | null } = { current: null };

    constructor(blockId: string) {
        this.blockId = blockId;
        this.viewIcon = createSignalAtom("shapes");
        this.viewName = createSignalAtom("Widget Launcher");
        this.noHeader = createSignalAtom(true);
        this.searchTerm = createSignalAtom("");
        this.selectedIndex = createSignalAtom(0);
        this.containerSize = createSignalAtom({ width: 0, height: 0 });
    }

    filteredWidgets(): WidgetConfigType[] {
        const searchTerm = this.searchTerm();
        const widgets = sortByDisplayOrder(atoms.fullConfigAtom()?.widgets || {});
        return widgets.filter(
            (widget) =>
                !widget["display:hidden"] &&
                (!searchTerm || widget.label?.toLowerCase().includes(searchTerm.toLowerCase()))
        );
    }

    giveFocus(): boolean {
        if (this.inputRef.current) {
            this.inputRef.current.focus();
            return true;
        }
        return false;
    }

    keyDownHandler(e: WaveKeyboardEvent): boolean {
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
        if (checkKeyPressed(e, "Enter")) {
            if (filteredWidgets.length == 0) return true;
            if (filteredWidgets[selectedIndex]) this.handleWidgetSelect(filteredWidgets[selectedIndex]);
            return true;
        }
        if (checkKeyPressed(e, "Escape")) {
            this.searchTerm._set("");
            this.selectedIndex._set(0);
            return true;
        }
        return false;
    }

    async handleWidgetSelect(widget: WidgetConfigType) {
        try {
            await replaceBlock(this.blockId, widget.blockdef, true);
        } catch (error) {
            console.error("Error replacing block:", error);
        }
    }
}

function LauncherView(props: ViewComponentProps<LauncherViewModel>): JSX.Element {
    const model = props.model;
    const searchTerm = model.searchTerm;
    const selectedIndex = model.selectedIndex;
    const filteredWidgets = createMemo(() => model.filteredWidgets());
    const containerSize = model.containerSize;

    let containerRef!: HTMLDivElement;
    let inputRef!: HTMLInputElement;

    onMount(() => {
        model.inputRef.current = inputRef;
        if (!containerRef) return;
        const resizeObserver = new ResizeObserver((entries) => {
            for (const entry of entries) {
                containerSize._set({
                    width: entry.contentRect.width,
                    height: entry.contentRect.height,
                });
            }
        });
        resizeObserver.observe(containerRef);
        return () => resizeObserver.disconnect();
    });

    const GAP = 16;
    const LABEL_THRESHOLD = 60;
    const MARGIN_BOTTOM = 24;
    const MAX_TILE_SIZE = 120;

    const calculatedLogoWidth = createMemo(() => containerSize().width * 0.3);
    const logoWidth = createMemo(() =>
        containerSize().width >= 100 ? Math.min(Math.max(calculatedLogoWidth(), 100), 300) : 0
    );
    const showLogo = createMemo(() => logoWidth() >= 100);
    const availableHeight = createMemo(
        () => containerSize().height - (showLogo() ? logoWidth() + MARGIN_BOTTOM : 0)
    );

    const gridLayout = createMemo<GridLayoutType>(() => {
        const cw = containerSize().width;
        const ah = availableHeight();
        const fw = filteredWidgets();
        if (cw === 0 || ah <= 0 || fw.length === 0) {
            return { columns: 1, tileWidth: 90, tileHeight: 90, showLabel: true };
        }
        let bestColumns = 1;
        let bestTileSize = 0;
        let bestTileWidth = 90;
        let bestTileHeight = 90;
        let showLabel = true;
        for (let cols = 1; cols <= fw.length; cols++) {
            const rows = Math.ceil(fw.length / cols);
            const tileWidth = (cw - (cols - 1) * GAP) / cols;
            const tileHeight = (ah - (rows - 1) * GAP) / rows;
            const currentTileSize = Math.min(tileWidth, tileHeight);
            if (currentTileSize > bestTileSize) {
                bestTileSize = currentTileSize;
                bestColumns = cols;
                bestTileWidth = tileWidth;
                bestTileHeight = tileHeight;
                showLabel = tileHeight >= LABEL_THRESHOLD;
            }
        }
        return { columns: bestColumns, tileWidth: bestTileWidth, tileHeight: bestTileHeight, showLabel };
    });

    // Keep model.gridLayout in sync
    createEffect(() => {
        model.gridLayout = gridLayout();
    });

    const finalTileWidth = createMemo(() => Math.min(gridLayout().tileWidth, MAX_TILE_SIZE));
    const finalTileHeight = createMemo(() =>
        gridLayout().showLabel ? Math.min(gridLayout().tileHeight, MAX_TILE_SIZE) : finalTileWidth()
    );

    // Reset selection when search term changes
    createEffect(() => {
        searchTerm(); // track
        selectedIndex._set(0);
    });

    return (
        <div ref={containerRef!} class="w-full h-full p-4 box-border flex flex-col items-center justify-center">
            {/* Hidden input for search */}
            <input
                ref={(el) => { inputRef = el; model.inputRef.current = el; }}
                type="text"
                value={searchTerm()}
                onKeyDown={keydownWrapper(model.keyDownHandler.bind(model))}
                onChange={(e) => model.searchTerm._set((e.target as HTMLInputElement).value)}
                class="sr-only dummy"
                aria-label="Search widgets"
            />

            {/* Logo */}
            <Show when={showLogo()}>
                <div class="mb-6" style={{ width: `${logoWidth()}px`, "max-width": "300px" }}>
                    <img src={logoUrl} class="w-full h-auto filter grayscale brightness-70 opacity-70" alt="Logo" />
                </div>
            </Show>

            {/* Grid of widgets */}
            <div
                class="grid gap-4 justify-center"
                style={{ "grid-template-columns": `repeat(${gridLayout().columns}, ${finalTileWidth()}px)` }}
            >
                <For each={filteredWidgets()}>
                    {(widget, index) => (
                        <div
                            onClick={() => model.handleWidgetSelect(widget)}
                            title={widget.description || widget.label}
                            class={clsx(
                                "flex flex-col items-center justify-center cursor-pointer rounded-md p-2 text-center",
                                "transition-colors duration-150",
                                index() === selectedIndex()
                                    ? "bg-white/20 text-white"
                                    : "bg-white/5 hover:bg-white/10 text-secondary hover:text-white"
                            )}
                            style={{ width: `${finalTileWidth()}px`, height: `${finalTileHeight()}px` }}
                        >
                            <div style={{ color: widget.color }}>
                                <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })} />
                            </div>
                            <Show when={gridLayout().showLabel && !isBlank(widget.label)}>
                                <div class="mt-1 w-full text-[11px] leading-4 overflow-hidden text-ellipsis whitespace-nowrap">
                                    {widget.label}
                                </div>
                            </Show>
                        </div>
                    )}
                </For>
            </div>

            {/* Search instructions */}
            <div class="mt-4 text-secondary text-xs">
                <Show
                    when={filteredWidgets().length === 0}
                    fallback={
                        <span>
                            {searchTerm() == "" ? "Type to Filter" : `Searching "${searchTerm()}"`}, Enter to Launch,
                            <Show when={searchTerm() == ""}>{" "}Arrow Keys to Navigate</Show>
                        </span>
                    }
                >
                    <span>No widgets found. Press Escape to clear search.</span>
                </Show>
            </div>
        </div>
    );
}

export default LauncherView;
