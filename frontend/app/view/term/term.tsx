// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsPrefixAtom, globalStore, pushNotification, WOS } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/waveutil";
import { ISearchOptions } from "@xterm/addon-search";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { TermStickers } from "./termsticker";
import { TermThemeUpdater } from "./termtheme";
import { computeTheme } from "./termutil";
import { makeTerminalModel, setTerminalViewComponent, TermViewModel } from "./termViewModel";
import { TermWrap } from "./termwrap";
import { DragOverlay } from "@/app/element/dragoverlay";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

// TermResyncHandler: watches connection status changes and resyncs the terminal controller.
function TermResyncHandler(props: { blockId: string; model: TermViewModel }): JSX.Element {
    const connStatus = createMemo(() => props.model.connStatus());

    let lastConnStatus: ConnStatus = connStatus();

    createEffect(() => {
        const cs = connStatus();
        if (!props.model.termRef.current?.hasResized) {
            lastConnStatus = cs;
            return;
        }
        const isConnected = cs?.status == "connected";
        const wasConnected = lastConnStatus?.status == "connected";
        const curConnName = cs?.connection;
        const lastConnName = lastConnStatus?.connection;
        if (isConnected == wasConnected && curConnName == lastConnName) {
            lastConnStatus = cs;
            return;
        }
        props.model.termRef.current?.resyncController("resync handler");
        lastConnStatus = cs;
    });

    return null;
}

function TerminalView(props: ViewComponentProps<TermViewModel>): JSX.Element {
    const { blockId, model } = props;
    let viewRef!: HTMLDivElement;
    let connectElemRef!: HTMLDivElement;
    let scrollbarHideObserverRef!: HTMLDivElement;

    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", blockId));
    const termSettingsAtom = getSettingsPrefixAtom("term");
    const termSettings = createMemo(() => termSettingsAtom());
    const termMode = createMemo(() => blockData()?.meta?.["term:mode"] ?? "term");
    const termFontSize = createMemo(() => model.fontSizeAtom());
    const isFocused = createMemo(() => model.nodeModel.isFocused());
    const isMI = createMemo(() => atoms.isTermMultiInput());
    const isBasicTerm = createMemo(() => blockData()?.meta?.controller != "cmd");

    // We use a ref-holder object that useSearch captures, so we can populate it after mount
    const anchorHolder = { current: null as HTMLDivElement | null };

    // search
    const searchProps = useSearch({
        anchorRef: anchorHolder,
        viewModel: model,
        caseSensitive: false,
        wholeWord: false,
        regex: false,
    });

    onMount(() => {
        anchorHolder.current = viewRef;
    });

    const searchIsOpen = createMemo(() => searchProps.isOpen?.() ?? false);
    const caseSensitive = createMemo(() => searchProps.caseSensitive?.() ?? false);
    const wholeWord = createMemo(() => searchProps.wholeWord?.() ?? false);
    const regex = createMemo(() => searchProps.regex?.() ?? false);
    const searchVal = createMemo(() => searchProps.searchValue?.() ?? "");

    const searchDecorations = {
        matchOverviewRuler: "#000000",
        activeMatchColorOverviewRuler: "#000000",
        activeMatchBorder: "#FF9632",
        matchBorder: "#FFFF00",
    };

    const searchOpts = createMemo<ISearchOptions>(() => ({
        regex: regex(),
        wholeWord: wholeWord(),
        caseSensitive: caseSensitive(),
        decorations: searchDecorations,
    }));

    const handleSearchError = (e: Error) => {
        console.warn("search error:", e);
    };

    const executeSearch = (searchText: string, direction: "next" | "previous") => {
        if (searchText === "") {
            model.termRef.current?.searchAddon.clearDecorations();
            return;
        }
        try {
            model.termRef.current?.searchAddon[direction === "next" ? "findNext" : "findPrevious"](
                searchText,
                searchOpts()
            );
        } catch (e) {
            handleSearchError(e as Error);
        }
    };

    searchProps.onSearch = (searchText: string) => executeSearch(searchText, "previous");
    searchProps.onPrev = () => executeSearch(searchVal(), "previous");
    searchProps.onNext = () => executeSearch(searchVal(), "next");

    // Return focus to terminal when search closes
    createEffect(() => {
        if (!searchIsOpen()) {
            model.giveFocus();
        }
    });

    // Re-run search when search opts change
    createEffect(() => {
        searchOpts(); // track
        model.termRef.current?.searchAddon.clearDecorations();
        if (searchProps.onSearch) searchProps.onSearch(searchVal());
    });

    // Initialize terminal
    onMount(() => {
        const fullConfig = atoms.fullConfigAtom();
        const connFontFamily = (fullConfig as any)?.connections?.[blockData()?.meta?.connection]?.["term:fontfamily"];
        const termThemeName = model.termThemeNameAtom();
        const termTransparency = model.termTransparencyAtom();
        const termBPMAtom = getOverrideConfigAtom(blockId, "term:allowbracketedpaste");
        const [termTheme] = computeTheme(fullConfig, termThemeName, termTransparency);
        const ts = termSettings();
        let termScrollback = 2000;
        if (ts?.["term:scrollback"]) termScrollback = Math.floor(ts["term:scrollback"]);
        if (blockData()?.meta?.["term:scrollback"]) termScrollback = Math.floor(blockData().meta["term:scrollback"]);
        termScrollback = Math.max(0, Math.min(termScrollback, 50000));
        const termAllowBPM = termBPMAtom() ?? false;
        const wasFocused = model.termRef.current != null && model.nodeModel.isFocused();
        const termWrap = new TermWrap(
            blockId,
            connectElemRef,
            {
                theme: termTheme,
                fontSize: termFontSize(),
                fontFamily: ts?.["term:fontfamily"] ?? connFontFamily ?? "Hack",
                drawBoldTextInBrightColors: false,
                fontWeight: "normal",
                fontWeightBold: "bold",
                allowTransparency: true,
                scrollback: termScrollback,
                allowProposedApi: true,
                ignoreBracketedPasteMode: !termAllowBPM,
            },
            {
                keydownHandler: model.handleTerminalKeydown.bind(model),
                useWebGl: !ts?.["term:disablewebgl"],
                sendDataHandler: model.sendDataToController.bind(model),
            }
        );
        (window as any).term = termWrap;
        model.termRef.current = termWrap;
        const rszObs = new ResizeObserver(() => {
            termWrap.handleResize_debounced();
        });
        rszObs.observe(connectElemRef);
        termWrap.onSearchResultsDidChange = (results: { resultIndex: number; resultCount: number }) => {
            if (searchProps.resultsIndex) globalStore.set(searchProps.resultsIndex, results.resultIndex);
            if (searchProps.resultsCount) globalStore.set(searchProps.resultsCount, results.resultCount);
        };
        fireAndForget(() => termWrap.init());
        if (wasFocused) {
            setTimeout(() => model.giveFocus(), 10);
        }
        onCleanup(() => {
            termWrap.dispose();
            rszObs.disconnect();
        });
    });

    // Update font size in-place when zoom changes
    createEffect(() => {
        const fs = termFontSize();
        const termWrap = model.termRef.current;
        if (termWrap?.terminal && termWrap.loaded) {
            termWrap.terminal.options.fontSize = fs;
            termWrap.handleResize();
        }
    });

    // Multi-input callback
    createEffect(() => {
        const mi = isMI();
        const bt = isBasicTerm();
        const focused = isFocused();
        if (mi && bt && focused && model.termRef.current != null) {
            model.termRef.current.multiInputCallback = (data: string) => {
                model.multiInputHandler(data);
            };
        } else {
            if (model.termRef.current != null) {
                model.termRef.current.multiInputCallback = null;
            }
        }
    });

    const onScrollbarShowObserver = () => {
        const termViewport = viewRef.getElementsByClassName("xterm-viewport")[0] as HTMLDivElement;
        if (termViewport) termViewport.style.zIndex = "var(--zindex-xterm-viewport-overlay)";
        if (scrollbarHideObserverRef) scrollbarHideObserverRef.style.display = "block";
    };

    const onScrollbarHideObserver = () => {
        const termViewport = viewRef.getElementsByClassName("xterm-viewport")[0] as HTMLDivElement;
        if (termViewport) termViewport.style.zIndex = "auto";
        if (scrollbarHideObserverRef) scrollbarHideObserverRef.style.display = "none";
    };

    const stickerConfig = createMemo(() => ({
        charWidth: 8,
        charHeight: 16,
        rows: model.termRef.current?.terminal?.rows ?? 24,
        cols: model.termRef.current?.terminal?.cols ?? 80,
        blockId: blockId,
    }));

    const termBg = createMemo(() => computeBgStyleFromMeta(blockData()?.meta));

    const handleFilesDropped = (paths: string[]) => {
        const cwd = blockData()?.meta?.["cmd:cwd"];
        if (!cwd) {
            console.warn("[term-drop] No working directory detected, ignoring drop");
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Drop failed",
                message: "No working directory detected for this terminal pane.",
                timestamp: new Date().toISOString(),
                type: "warning",
                expiration: Date.now() + 8000,
            });
            return;
        }
        for (const filePath of paths) {
            const fileName = filePath.split(/[\\/]/).pop() ?? filePath;
            invoke("copy_file_to_dir", { sourcePath: filePath, targetDir: cwd })
                .then((destPath: any) => {
                    console.log(`[term-drop] copied ${fileName} → ${destPath}`);
                })
                .catch((err: any) => {
                    const msg = String(err);
                    pushNotification({
                        icon: "fa-triangle-exclamation",
                        title: `Copy failed: ${fileName}`,
                        message: msg,
                        timestamp: new Date().toISOString(),
                        type: "error",
                        expiration: Date.now() + 12000,
                    });
                });
        }
    };

    // File drop via Tauri's window-level event (since HTML5 drag events don't fire for OS drops in WebView2)
    const [isDragOver, setIsDragOver] = createSignal(false);

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getCurrentWebview().onDragDropEvent((event) => {
            const type = event.payload.type;
            const pos = (event.payload as any).position as { x: number; y: number } | undefined;
            const isOverEl = () => {
                if (!pos || !viewRef) return false;
                const rect = viewRef.getBoundingClientRect();
                return pos.x >= rect.left && pos.x <= rect.right && pos.y >= rect.top && pos.y <= rect.bottom;
            };
            if (type === "over") {
                setIsDragOver(isOverEl());
            } else if (type === "drop") {
                setIsDragOver(false);
                if (!isOverEl()) return;
                const paths = (event.payload as any).paths as string[] | undefined;
                if (paths && paths.length > 0) {
                    handleFilesDropped(paths);
                }
            } else if (type === "leave" || (type as string) === "cancel") {
                setIsDragOver(false);
            }
        }).then((fn) => {
            unlisten = fn;
        });
        onCleanup(() => unlisten?.());
    });

    const dropMessage = createMemo(() => {
        const cwd = blockData()?.meta?.["cmd:cwd"];
        return cwd ? `Copy to ${cwd}` : "No working directory detected";
    });

    return (
        <div
            ref={viewRef!}
            class={clsx("view-term", "term-mode-" + termMode())}
            style={{ position: "relative" }}
        >
            <DragOverlay message={dropMessage()} visible={isDragOver()} />
            <Show when={termBg()}>
                <div class="absolute inset-0 z-0 pointer-events-none" style={termBg()} />
            </Show>
            <TermResyncHandler blockId={blockId} model={model} />
            <TermThemeUpdater blockId={blockId} model={model} termRef={model.termRef} />
            <TermStickers config={stickerConfig()} />
            <div class="term-connectelem" ref={connectElemRef!}>
                <div class="term-scrollbar-show-observer" onPointerOver={onScrollbarShowObserver} />
                <div
                    ref={scrollbarHideObserverRef!}
                    class="term-scrollbar-hide-observer"
                    onPointerOver={onScrollbarHideObserver}
                />
            </div>
            <Search {...searchProps} />
        </div>
    );
}

// Register TerminalView with the ViewModel to break the circular dependency
setTerminalViewComponent(TerminalView);

export { makeTerminalModel, TerminalView, TermViewModel };
