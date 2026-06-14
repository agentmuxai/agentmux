// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsKeyAtom, getSettingsPrefixAtom, pushNotification, WOS } from "@/store/global";
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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { baseName, consumeDragPaths, copyFilesToDir } from "@/util/dnd";

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
            if (searchProps.resultsIndex) searchProps.resultsIndex._set(results.resultIndex);
            if (searchProps.resultsCount) searchProps.resultsCount._set(results.resultCount);
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

    // Ctrl+Wheel zoom: capture phase so we intercept before xterm's bubble-phase
    // wheel listener. stopPropagation() prevents xterm from scrolling the buffer.
    // preventDefault() suppresses CEF's native Ctrl+Scroll page zoom.
    onMount(() => {
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const currentZoom = model.termZoomAtom();
            const STEP = 0.1;
            const delta = ev.deltaY > 0 ? -STEP : STEP; // scroll down = zoom out
            const next = Math.max(0.5, Math.min(2.0, Math.round((currentZoom + delta) * 100) / 100));
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", blockId),
                meta: { "term:zoom": next === 1.0 ? null : next },
            });
        };
        viewRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => viewRef.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
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

    const stickerConfig = createMemo(() => ({
        charWidth: 8,
        charHeight: 16,
        rows: model.termRef.current?.terminal?.rows ?? 24,
        cols: model.termRef.current?.terminal?.cols ?? 80,
        blockId: blockId,
    }));

    const termBg = createMemo(() => computeBgStyleFromMeta(blockData()?.meta));

    const dndEnabledAtom = getSettingsKeyAtom("dnd:enabled");
    const dndConcurrencyAtom = getSettingsKeyAtom("dnd:concurrency");
    const dndEnabled = () => (dndEnabledAtom() ?? true) !== false;
    const dndConcurrency = () => {
        const v = dndConcurrencyAtom();
        return typeof v === "number" && v > 0 ? v : undefined;
    };

    const handleFilesDropped = async (paths: string[]) => {
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
        const outcome = await copyFilesToDir(paths, cwd, { concurrency: dndConcurrency() });
        const successes = outcome.results.filter((r) => r.dest);
        const failures = outcome.results.filter((r) => r.error);
        if (successes.length > 0) {
            const summary =
                successes.length === 1
                    ? `Copied ${baseName(successes[0].dest!)} to ${cwd}`
                    : `Copied ${successes.length} files to ${cwd}`;
            pushNotification({
                icon: "fa-check",
                title: failures.length > 0 ? `${summary} (${failures.length} failed)` : summary,
                message: failures.length > 0 ? failures.map((f) => `${baseName(f.source)}: ${f.error}`).join("\n") : "",
                timestamp: new Date().toISOString(),
                type: failures.length > 0 ? "warning" : "info",
                expiration: Date.now() + 6000,
            });
        } else if (failures.length > 0) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: `Copy failed (${failures.length} file${failures.length === 1 ? "" : "s"})`,
                message: failures.map((f) => `${baseName(f.source)}: ${f.error}`).join("\n"),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 12000,
            });
        }
    };

    const [isDragOver, setIsDragOver] = createSignal(false);

    onMount(() => {
        if (detectHost() === "cef") {
            // CEF: HTML5 drag events work natively (unlike WebView2)
            if (!viewRef) return;
            const onDragOver = (e: DragEvent) => {
                if (!dndEnabled()) return;
                // Only treat file drags as drop targets — text/URL drags keep
                // their browser default behavior so a selection or link dragged
                // over a terminal doesn't trigger a misleading "Copy to <cwd>"
                // overlay. Matches the guard in useAgentDropAttach.
                const types = e.dataTransfer?.types;
                if (!types || !Array.from(types).includes("Files")) return;
                e.preventDefault();
                setIsDragOver(true);
            };
            const onDragLeave = (e: DragEvent) => {
                // Only clear when the drag actually leaves the pane — see the
                // matching comment in useAgentDropAttach. xterm fills the pane
                // with composited child layers; treating every dragleave as
                // "drag is gone" caused the overlay to flicker the moment the
                // cursor crossed into the xterm viewport.
                const next = e.relatedTarget as Node | null;
                if (!next || !viewRef.contains(next)) setIsDragOver(false);
            };
            const onDrop = (e: DragEvent) => {
                if (!dndEnabled()) return;
                e.preventDefault();
                setIsDragOver(false);
                const files = e.dataTransfer?.files;
                if (!files || files.length === 0) return;
                // HTML5 File API only exposes bare filenames; the OS paths
                // were captured by CefDragHandler::on_drag_enter and stashed
                // in the host. Consume the stash now — it's keyed by drag
                // session, not by pane, so it returns the same N paths the
                // browser sees as `files`.
                void consumeDragPaths().then((paths) => {
                    if (paths.length > 0) {
                        handleFilesDropped(paths);
                        return;
                    }
                    // Stash empty: TTL expired, or the OnDragEnter callback
                    // didn't fire (e.g. browser pane child window that
                    // doesn't carry our DragHandler). Surface a clear
                    // message rather than silently dropping.
                    pushNotification({
                        icon: "fa-triangle-exclamation",
                        title: "Drop failed",
                        message: `Couldn't read the OS paths for ${files.length} dropped file(s). Try again.`,
                        timestamp: new Date().toISOString(),
                        type: "warning",
                        expiration: Date.now() + 6000,
                    });
                });
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
            <div class="term-connectelem" ref={connectElemRef!} />
            <Search {...searchProps} />
        </div>
    );
}

// Register TerminalView with the ViewModel to break the circular dependency
setTerminalViewComponent(TerminalView);

export { makeTerminalModel, TerminalView, TermViewModel };
