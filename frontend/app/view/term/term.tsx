// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsPrefixAtom, pushNotification, WOS } from "@/store/global";
import { backendStatusAtom } from "@/store/backendStatus";
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
import "./xterm.css";
import { DragOverlay } from "@/app/element/dragoverlay";
import { detectHost, invokeCommand } from "@/app/platform/ipc";

// TermResyncHandler: watches connection status changes and resyncs the terminal controller.
// Also resyncs when the backend restarts — local terminals have no connStatus change on restart,
// so without this the existing PTY stays dead after reconnect even though "running" is shown.
function TermResyncHandler(props: { blockId: string; model: TermViewModel }): JSX.Element {
    const connStatus = createMemo(() => props.model.connStatus());

    let lastConnStatus: ConnStatus = connStatus();
    let lastBackendStatus = backendStatusAtom();

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

    // Resync when backend transitions to "running" after a restart.
    // Catches the case where the sidecar crashed and came back — the PTY is gone
    // but connStatus for local terminals never changes, so the effect above never fires.
    createEffect(() => {
        const bs = backendStatusAtom();
        if (bs === "running" && lastBackendStatus !== "running") {
            props.model.termRef.current?.resyncController("backend-restart");
        }
        lastBackendStatus = bs;
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
        window.term = termWrap;
        model.termRef.current = termWrap;
        const rszObs = new ResizeObserver(() => {
            termWrap.handleResize_debounced();
        });
        rszObs.observe(connectElemRef);
        termWrap.onSearchResultsDidChange = (results: { resultIndex: number; resultCount: number }) => {
            if (searchProps.resultsIndex) searchProps.resultsIndex(results.resultIndex);
            if (searchProps.resultsCount) searchProps.resultsCount(results.resultCount);
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
        // xterm v6 reworked the viewport/scrollbar — try both old and new class names
        const termViewport = (viewRef.getElementsByClassName("xterm-viewport")[0] ??
            viewRef.getElementsByClassName("xterm-scroll-area")[0]) as HTMLDivElement;
        if (termViewport) termViewport.style.zIndex = "var(--zindex-xterm-viewport-overlay)";
        if (scrollbarHideObserverRef) scrollbarHideObserverRef.style.display = "block";
    };

    const onScrollbarHideObserver = () => {
        const termViewport = (viewRef.getElementsByClassName("xterm-viewport")[0] ??
            viewRef.getElementsByClassName("xterm-scroll-area")[0]) as HTMLDivElement;
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
            invokeCommand("copy_file_to_dir", { sourcePath: filePath, targetDir: cwd })
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
        if (detectHost() === "tauri") {
            // Tauri: file drop via window-level event (HTML5 drag events don't fire in WebView2)
            let unlisten: (() => void) | null = null;
            import("@tauri-apps/api/webview").then(({ getCurrentWebview }) => {
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
            });
            onCleanup(() => unlisten?.());
        } else if (detectHost() === "cef") {
            // CEF: HTML5 drag events work natively (unlike WebView2)
            if (!viewRef) return;
            const onDragOver = (e: DragEvent) => {
                e.preventDefault();
                setIsDragOver(true);
            };
            const onDragLeave = () => setIsDragOver(false);
            const onDrop = (e: DragEvent) => {
                e.preventDefault();
                setIsDragOver(false);
                // HTML5 File API doesn't expose full paths — CEF needs CefDragHandler for that.
                // For now, log the file names as a placeholder.
                const files = e.dataTransfer?.files;
                if (files && files.length > 0) {
                    const names = Array.from(files).map(f => f.name);
                    console.log("[term-drop] CEF drop:", names.join(", "), "(full path copy not yet implemented)");
                    pushNotification({
                        icon: "fa-info-circle",
                        title: "File drop",
                        message: `Dropped ${files.length} file(s). Full path copy requires CefDragHandler integration.`,
                        timestamp: new Date().toISOString(),
                        type: "info",
                        expiration: Date.now() + 5000,
                    });
                }
            };
            viewRef.addEventListener("dragover", onDragOver);
            viewRef.addEventListener("dragleave", onDragLeave);
            viewRef.addEventListener("drop", onDrop);
            onCleanup(() => {
                viewRef.removeEventListener("dragover", onDragOver);
                viewRef.removeEventListener("dragleave", onDragLeave);
                viewRef.removeEventListener("drop", onDrop);
            });
        }
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
            <Show when={model.agentRuntimeLabel()}>
                <div class="agent-runtime-badge" title="Agent running time">
                    {model.agentRuntimeLabel()}
                </div>
            </Show>
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
