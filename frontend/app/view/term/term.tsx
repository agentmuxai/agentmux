// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsPrefixAtom, globalStore, pushNotification, WOS } from "@/store/global";
import { fireAndForget, useAtomValueSafe } from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/waveutil";
import { ISearchOptions } from "@xterm/addon-search";
import clsx from "clsx";
import * as jotai from "jotai";
import * as React from "react";
import { TermStickers } from "./termsticker";
import { TermThemeUpdater } from "./termtheme";
import { computeTheme } from "./termutil";
import { TermResyncHandler, TermToolbarVDomNode, TermVDomNode } from "./termVDom";
import { makeTerminalModel, setTerminalViewComponent, TermViewModel } from "./termViewModel";
import { TermWrap } from "./termwrap";
import "./xterm.css";
import { DragOverlay } from "@/app/element/dragoverlay";
import { useFileDrop } from "@/app/hook/useFileDrop";
import { invoke } from "@tauri-apps/api/core";

const TerminalView = ({ blockId, model }: ViewComponentProps<TermViewModel>) => {
    const viewRef = React.useRef<HTMLDivElement>(null);
    const connectElemRef = React.useRef<HTMLDivElement>(null);
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", blockId));
    const termSettingsAtom = getSettingsPrefixAtom("term");
    const termSettings = jotai.useAtomValue(termSettingsAtom);
    let termMode = blockData?.meta?.["term:mode"] ?? "term";
    if (termMode != "term" && termMode != "vdom") {
        termMode = "term";
    }
    const termModeRef = React.useRef(termMode);

    const termFontSize = jotai.useAtomValue(model.fontSizeAtom);
    const fullConfig = globalStore.get(atoms.fullConfigAtom);
    const connFontFamily = fullConfig.connections?.[blockData?.meta?.connection]?.["term:fontfamily"];
    const isFocused = jotai.useAtomValue(model.nodeModel.isFocused);
    const isMI = jotai.useAtomValue(atoms.isTermMultiInput);
    const isBasicTerm = termMode != "vdom" && blockData?.meta?.controller != "cmd"; // needs to match isBasicTerm

    // search
    const searchProps = useSearch({
        anchorRef: viewRef,
        viewModel: model,
        caseSensitive: false,
        wholeWord: false,
        regex: false,
    });
    const searchIsOpen = jotai.useAtomValue<boolean>(searchProps.isOpen);
    const caseSensitive = useAtomValueSafe<boolean>(searchProps.caseSensitive);
    const wholeWord = useAtomValueSafe<boolean>(searchProps.wholeWord);
    const regex = useAtomValueSafe<boolean>(searchProps.regex);
    const searchVal = jotai.useAtomValue<string>(searchProps.searchValue);
    const searchDecorations = React.useMemo(
        () => ({
            matchOverviewRuler: "#000000",
            activeMatchColorOverviewRuler: "#000000",
            activeMatchBorder: "#FF9632",
            matchBorder: "#FFFF00",
        }),
        []
    );
    const searchOpts = React.useMemo<ISearchOptions>(
        () => ({
            regex,
            wholeWord,
            caseSensitive,
            decorations: searchDecorations,
        }),
        [regex, wholeWord, caseSensitive]
    );
    const handleSearchError = React.useCallback((e: Error) => {
        console.warn("search error:", e);
    }, []);
    const executeSearch = React.useCallback(
        (searchText: string, direction: "next" | "previous") => {
            if (searchText === "") {
                model.termRef.current?.searchAddon.clearDecorations();
                return;
            }
            try {
                model.termRef.current?.searchAddon[direction === "next" ? "findNext" : "findPrevious"](
                    searchText,
                    searchOpts
                );
            } catch (e) {
                handleSearchError(e);
            }
        },
        [searchOpts, handleSearchError]
    );
    searchProps.onSearch = React.useCallback(
        (searchText: string) => executeSearch(searchText, "previous"),
        [executeSearch]
    );
    searchProps.onPrev = React.useCallback(() => executeSearch(searchVal, "previous"), [executeSearch, searchVal]);
    searchProps.onNext = React.useCallback(() => executeSearch(searchVal, "next"), [executeSearch, searchVal]);
    // Return input focus to the terminal when the search is closed
    React.useEffect(() => {
        if (!searchIsOpen) {
            model.giveFocus();
        }
    }, [searchIsOpen]);
    // rerun search when the searchOpts change
    React.useEffect(() => {
        model.termRef.current?.searchAddon.clearDecorations();
        searchProps.onSearch(searchVal);
    }, [searchOpts]);
    // end search

    React.useEffect(() => {
        const fullConfig = globalStore.get(atoms.fullConfigAtom);
        const termThemeName = globalStore.get(model.termThemeNameAtom);
        const termTransparency = globalStore.get(model.termTransparencyAtom);
        const termBPMAtom = getOverrideConfigAtom(blockId, "term:allowbracketedpaste");
        const [termTheme, _] = computeTheme(fullConfig, termThemeName, termTransparency);
        let termScrollback = 2000;
        if (termSettings?.["term:scrollback"]) {
            termScrollback = Math.floor(termSettings["term:scrollback"]);
        }
        if (blockData?.meta?.["term:scrollback"]) {
            termScrollback = Math.floor(blockData.meta["term:scrollback"]);
        }
        if (termScrollback < 0) {
            termScrollback = 0;
        }
        if (termScrollback > 50000) {
            termScrollback = 50000;
        }
        const termAllowBPM = globalStore.get(termBPMAtom) ?? false;
        const wasFocused = model.termRef.current != null && globalStore.get(model.nodeModel.isFocused);
        const termWrap = new TermWrap(
            blockId,
            connectElemRef.current,
            {
                theme: termTheme,
                fontSize: termFontSize,
                fontFamily: termSettings?.["term:fontfamily"] ?? connFontFamily ?? "Hack",
                drawBoldTextInBrightColors: false,
                fontWeight: "normal",
                fontWeightBold: "bold",
                allowTransparency: true,
                scrollback: termScrollback,
                allowProposedApi: true, // Required by @xterm/addon-search to enable search functionality and decorations
                ignoreBracketedPasteMode: !termAllowBPM,
            },
            {
                keydownHandler: model.handleTerminalKeydown.bind(model),
                useWebGl: !termSettings?.["term:disablewebgl"],
                sendDataHandler: model.sendDataToController.bind(model),
            }
        );
        (window as any).term = termWrap;
        model.termRef.current = termWrap;
        const rszObs = new ResizeObserver(() => {
            termWrap.handleResize_debounced();
        });
        rszObs.observe(connectElemRef.current);
        termWrap.onSearchResultsDidChange = (results) => {
            globalStore.set(searchProps.resultsIndex, results.resultIndex);
            globalStore.set(searchProps.resultsCount, results.resultCount);
        };
        fireAndForget(() => termWrap.init());
        if (wasFocused) {
            setTimeout(() => {
                model.giveFocus();
            }, 10);
        }
        return () => {
            termWrap.dispose();
            rszObs.disconnect();
        };
    }, [blockId, termSettings, connFontFamily]);

    // Update font size in-place when zoom changes (avoids full terminal reconstruction
    // which would destroy/recreate WebGL contexts and cause other panes to lose content)
    React.useEffect(() => {
        const termWrap = model.termRef.current;
        if (termWrap?.terminal && termWrap.loaded) {
            termWrap.terminal.options.fontSize = termFontSize;
            termWrap.handleResize();
        }
    }, [termFontSize]);

    React.useEffect(() => {
        if (termModeRef.current == "vdom" && termMode == "term") {
            // focus the terminal
            model.giveFocus();
        }
        termModeRef.current = termMode;
    }, [termMode]);

    React.useEffect(() => {
        if (isMI && isBasicTerm && isFocused && model.termRef.current != null) {
            model.termRef.current.multiInputCallback = (data: string) => {
                model.multiInputHandler(data);
            };
        } else {
            if (model.termRef.current != null) {
                model.termRef.current.multiInputCallback = null;
            }
        }
    }, [isMI, isBasicTerm, isFocused]);

    const scrollbarHideObserverRef = React.useRef<HTMLDivElement>(null);
    const onScrollbarShowObserver = React.useCallback(() => {
        const termViewport = viewRef.current.getElementsByClassName("xterm-viewport")[0] as HTMLDivElement;
        termViewport.style.zIndex = "var(--zindex-xterm-viewport-overlay)";
        scrollbarHideObserverRef.current.style.display = "block";
    }, []);
    const onScrollbarHideObserver = React.useCallback(() => {
        const termViewport = viewRef.current.getElementsByClassName("xterm-viewport")[0] as HTMLDivElement;
        termViewport.style.zIndex = "auto";
        scrollbarHideObserverRef.current.style.display = "none";
    }, []);

    const stickerConfig = {
        charWidth: 8,
        charHeight: 16,
        rows: model.termRef.current?.terminal.rows ?? 24,
        cols: model.termRef.current?.terminal.cols ?? 80,
        blockId: blockId,
    };

    const termBg = computeBgStyleFromMeta(blockData?.meta);

    const handleFilesDropped = React.useCallback(
        (paths: string[]) => {
            const cwd = blockData?.meta?.["cmd:cwd"];
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
                console.log(`[term-drop] copying ${filePath} → ${cwd}`);
                invoke("copy_file_to_dir", { sourcePath: filePath, targetDir: cwd })
                    .then((destPath: any) => {
                        console.log(`[term-drop] ✓ copied ${fileName} → ${destPath}`);
                    })
                    .catch((err: any) => {
                        const msg = String(err);
                        console.warn(`[term-drop] ✗ failed: ${msg}`);
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
        },
        [blockData]
    );

    const { isDragOver, handlers: dropHandlers } = useFileDrop(handleFilesDropped, viewRef);
    const cwd = blockData?.meta?.["cmd:cwd"];
    const dropMessage = cwd ? `Copy to ${cwd}` : "No working directory detected";

    React.useEffect(() => {
        if (isDragOver) {
            console.log("[dnd-debug] drag over block", blockId, "— full meta:", JSON.stringify(blockData?.meta ?? {}));
            console.log("[dnd-debug] cmd:cwd =", cwd ?? "(undefined)");
        }
    }, [isDragOver]);

    return (
        <div className={clsx("view-term", "term-mode-" + termMode)} ref={viewRef} {...dropHandlers} style={{ position: "relative" }}>
            <DragOverlay message={dropMessage} visible={isDragOver} />
            {termBg && <div className="absolute inset-0 z-0 pointer-events-none" style={termBg} />}
            <TermResyncHandler blockId={blockId} model={model} />
            <TermThemeUpdater blockId={blockId} model={model} termRef={model.termRef} />
            <TermStickers config={stickerConfig} />
            <TermToolbarVDomNode key="vdom-toolbar" blockId={blockId} model={model} />
            <TermVDomNode key="vdom" blockId={blockId} model={model} />
            <div key="conntectElem" className="term-connectelem" ref={connectElemRef}>
                <div className="term-scrollbar-show-observer" onPointerOver={onScrollbarShowObserver} />
                <div
                    ref={scrollbarHideObserverRef}
                    className="term-scrollbar-hide-observer"
                    onPointerOver={onScrollbarHideObserver}
                />
            </div>
            <Search {...searchProps} />
        </div>
    );
};

// Register TerminalView with the ViewModel to break the circular dependency
setTerminalViewComponent(TerminalView);

export { makeTerminalModel, TerminalView, TermViewModel };
