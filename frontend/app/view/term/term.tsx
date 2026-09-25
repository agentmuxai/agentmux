// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsKeyAtom, getSettingsPrefixAtom, pushNotification, useBlockAtom, MOS } from "@/store/global";
import { backendStatusAtom } from "@/store/backendStatus";
import { fireAndForget } from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/muxutil";
import { ISearchOptions } from "@xterm/addon-search";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { resolveTermFontFamily } from "./termfontfamily";
import { resolveTermScrollback } from "./termscrollback";
import { TermStickers } from "./termsticker";
import { TermThemeUpdater } from "./termtheme";
import { computeTheme } from "./termutil";
import { setTermPaneChromeModel, setTerminalViewComponent, TermViewModel } from "./termViewModel";
import { TermWrap } from "./termwrap";
import "./xterm.css";
import { DragOverlay } from "@/app/element/dragoverlay";
import { detectHost, invokeCommand } from "@/app/platform/ipc";
import { focusManager } from "@/app/store/focusManager";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { baseName, consumeDragPaths, copyFilesToDir } from "@/util/dnd";
import type { NodeModel } from "@/layout/index";
import { ErrorBoundary } from "@/element/errorboundary";
import { createSignalAtom } from "@/util/util";
import type { SignalAtom } from "@/util/util";

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

    const [blockData] = MOS.useMuxObjectValue<Block>(MOS.makeORef("block", blockId));

    const termSettingsAtom = getSettingsPrefixAtom("term");
    const termSettings = createMemo(() => termSettingsAtom());
    const termMode = createMemo(() => blockData()?.meta?.["term:mode"] ?? "term");
    const termFontSize = createMemo(() => model.fontSizeAtom());
    const termScrollSensitivity = createMemo(() => model.scrollSensitivityAtom());
    // Settings resolved once at TermWrap construction and never revisited
    // until now — see SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md
    // §6.1. Promoted from one-shot local `const`s inside onMount (below) to
    // top-level memos so onMount and the live-apply effects further down
    // share one source of truth instead of onMount re-deriving its own copy.
    const termFontFamily = createMemo(() => {
        const connFontFamily = (atoms.fullConfigAtom() as any)?.connections?.[blockData()?.meta?.connection]?.[
            "term:fontfamily"
        ];
        return resolveTermFontFamily(termSettings(), connFontFamily);
    });
    const termScrollbackDepth = createMemo(() => resolveTermScrollback(termSettings(), blockData()?.meta));
    // Default ON: modern shells (bash 4+, zsh, fish) all support BPM and it
    // prevents the shell from executing partial lines mid-paste. Disable
    // per-pane via term:allowbracketedpaste=false for legacy shells that
    // don't support it.
    const termAllowBracketedPaste = createMemo(() => getOverrideConfigAtom(blockId, "term:allowbracketedpaste")() ?? true);
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
        const termThemeName = model.termThemeNameAtom();
        const termTransparency = model.termTransparencyAtom();
        const [termTheme] = computeTheme(fullConfig, termThemeName, termTransparency);
        const ts = termSettings();
        const termWrap = new TermWrap(
            blockId,
            connectElemRef,
            {
                theme: termTheme,
                fontSize: termFontSize(),
                fontFamily: termFontFamily(),
                drawBoldTextInBrightColors: false,
                fontWeight: "normal",
                fontWeightBold: "bold",
                allowTransparency: true,
                scrollback: termScrollbackDepth(),
                allowProposedApi: true,
                ignoreBracketedPasteMode: !termAllowBracketedPaste(),
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
        // SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §2c: the old `wasFocused`
        // precheck above read `model.termRef.current` BEFORE the assignment
        // three lines up ever ran, so it was always false on a genuine first
        // mount — this pane's "already implemented" giveFocus() only ever
        // fired on a re-mount of an already-initialized model. It also used
        // `nodeModel.isFocused()`, which is true even for a pane sitting in a
        // BACKGROUND tab (that memo is scoped to the pane's own tab, not the
        // active one) — swapped for focusManager.claimFocusOnMount's
        // active-tab-scoped check so a terminal created out of view can't
        // steal focus from whatever the user is actually looking at.
        setTimeout(() => focusManager.claimFocusOnMount(blockId, () => model.giveFocus()), 10);
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
            // Ctrl+Shift+Scroll is AppAllPanesZoomHandler's all-panes gesture
            // (app.tsx) — let it bubble there instead of zooming just this
            // pane. See SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
            if (!ev.ctrlKey || ev.shiftKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const currentZoom = model.termZoomAtom();
            const STEP = 0.1;
            const delta = ev.deltaY > 0 ? -STEP : STEP; // scroll down = zoom out
            const next = Math.max(0.5, Math.min(2.0, Math.round((currentZoom + delta) * 100) / 100));
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: MOS.makeORef("block", blockId),
                meta: { "term:zoom": next === 1.0 ? null : next },
            });
        };
        viewRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => viewRef.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    // Update font size AND font family in-place when either changes — both
    // affect character-cell geometry (a font-family swap can change glyph
    // width same as a size change would), so both need handleResize() and
    // share one effect rather than each independently triggering it.
    createEffect(() => {
        const fs = termFontSize();
        const ff = termFontFamily();
        const termWrap = model.termRef.current;
        if (termWrap?.terminal && termWrap.loaded) {
            termWrap.terminal.options.fontSize = fs;
            termWrap.terminal.options.fontFamily = ff;
            termWrap.handleResize();
        }
    });

    // Update scroll sensitivity, scrollback depth, and bracketed-paste
    // mode in-place when any changes — all three were previously only
    // applied at Terminal construction, so changing any of them in
    // Settings had no effect on already-open panes until reopened. Grouped
    // into one effect: unlike font size/family above, none of these
    // affect cell geometry, so nothing here needs handleResize(). (theme
    // — the fourth setting audited alongside these — turned out to
    // already be live via termtheme.ts's TermThemeUpdater, so it isn't
    // here.) See SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md §6.1
    // (and REPORT_TERMINAL_SCROLL_SENSITIVITY_NOT_LIVE_2026_09_22.md for
    // scrollSensitivity specifically, the first of these fixed).
    createEffect(() => {
        const ss = termScrollSensitivity();
        const sb = termScrollbackDepth();
        const bpm = termAllowBracketedPaste();
        const termWrap = model.termRef.current;
        if (termWrap?.terminal && termWrap.loaded) {
            termWrap.terminal.options.scrollSensitivity = ss;
            termWrap.terminal.options.scrollback = sb;
            termWrap.terminal.options.ignoreBracketedPasteMode = !bpm;
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
            <TermResyncHandler blockId={blockId} model={model} />
            <TermThemeUpdater blockId={blockId} model={model} termRef={model.termRef} />
            <TermStickers config={stickerConfig()} />
            <div class="term-connectelem" ref={connectElemRef!} />
            <Search {...searchProps} />
        </div>
    );
}


/**
 * Chrome half of the terminal pane — the pane header and the in-pane tab
 * strip, hoisted OUT of `TerminalView` so neither is inside the per-block
 * remount boundary `pane-leaf-chrome.tsx` creates. Mounted once per leaf and
 * kept mounted across every subsequent tab switch; that persistence is the
 * whole point (`docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`).
 * Mirrors `AgentPaneChrome` (agent-view.tsx) — see that component for the
 * fuller commentary on the pattern; the terminal's version is simpler
 * (no progress-bar slot, no picker cross-fade, no pane-scope ModalLayer).
 *
 * `anchorBlockId` is whichever stack member's `TermViewModel` first called
 * `renderPaneChrome`; it is only a fallback for the active block id (that
 * member's tab can be closed while chrome lives on).
 */
/**
 * Terminal's opt-in to the ONE shared pane chrome
 * (`renderPaneChromeShell`) — see `PaneChromeModel` (custom.d.ts).
 * Everything the old `TermPaneChrome` component rendered around the
 * content (root box, focus ring, header row, tabs, ErrorBoundary) is the
 * shared chrome's job now; what stays here is only what is genuinely
 * terminal's: its connection button, the new-tab cwd, and the body wrapper
 * its background image / runtime badge need to span. Tab labels live in
 * term-pane-tab.ts. Focus hand-off on a tab switch is the host's, for every
 * tab type (block.tsx, Pane Tab contract Phase 3b).
 */
export function buildTermPaneChromeModel(anchorBlockId: string, nodeModel: NodeModel): PaneChromeModel {
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? anchorBlockId;

    // Tracks the CURRENTLY ACTIVE member, not the anchor — getMuxObjectAtom
    // inside a memo (not useMuxObjectValue), the reactive-oref pattern
    // established in #3134 for exactly this kind of switch-surviving reader.
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());

    // NOT stand-ins, unlike AgentPaneChrome's: TermViewModel sets
    // `manageConnection` to `!isCmd`, i.e. TRUE for every ordinary
    // terminal, so BlockFrame_Header really does render the connection
    // button here and it has to work. Both of these resolve the SAME
    // per-block objects BlockFrame_Default_Component itself uses (the
    // block-atom cache is keyed on blockId), which is what keeps the
    // hoisted button wired to the ChangeConnectionBlockModal that still
    // lives inside BlockFrame: the atom opens it, the ref anchors it.
    // Re-derived per active member so switching tabs targets the right
    // block's modal state.
    // Both read the ACTIVE member, so they follow tab switches: the
    // background is per-block meta, the runtime label is per-ViewModel
    // state owned by whichever TermViewModel is currently mounted.
    const termBg = createMemo(() => computeBgStyleFromMeta(activeBlockData()?.meta, null));
    const runtimeLabel = () => (nodeModel.activeViewModel?.() as TermViewModel | null)?.agentRuntimeLabel?.() ?? null;

    const changeConnModalAtom = createMemo(
        () => useBlockAtom(activeBlockId(), "changeConn", () => createSignalAtom(false)) as SignalAtom<boolean>,
    );
    const connBtnRef = createMemo(
        () =>
            useBlockAtom(activeBlockId(), "connBtnRef", () => {
                const holder: { current: HTMLDivElement | null } = { current: null };
                return () => holder;
            })(),
    );
    return {
        // No `onActivate`: a tab switch is the shared chrome's ordinary
        // stack switch, and the HOST hands focus to the newly visible tab
        // when the pane is focused, for every tab type (block.tsx, Pane Tab
        // contract Phase 3b).
        // term's ConnectionButton (manageConnection, TermViewModel) is a
        // real live feature BlockFrame_Header renders — threaded through
        // unchanged.
        connBtnRef: connBtnRef(),
        changeConnModalAtom: changeConnModalAtom(),
        // A new terminal tab inherits the CURRENT tab's cwd, matching how a
        // real terminal's "new tab" starts in the same directory. This used
        // to live in term's own `handleTermTabAdd`; it moved here when "+"
        // became the shared widget picker, so it applies to a terminal
        // added from this pane's picker and to nothing else (any other view
        // type gets undefined and is unaffected).
        newTabMeta: (view: string | undefined) => {
            if (view !== "term") return undefined;
            const cwd = activeBlockData()?.meta?.["cmd:cwd"] as string | undefined;
            // The same key `build_pane_meta`'s own "term" branch writes from
            // a top-level `cwd` arg (pane.rs) — set directly here because
            // this path always supplies `meta`, so that branch never runs.
            return cwd ? { "cmd:cwd": cwd } : undefined;
        },
        rootClass: "term-pane-stack",
        contentClass: "term-pane-stack-content",
        // The background image and runtime badge span the strip AND the
        // terminal, so they need a wrapper around the content region
        // rather than a slot beside it (term.scss's own
        // `> .term-pane-stack-body` positioning depends on this box).
        wrapContent: (content: JSX.Element) => (
            <div class="term-pane-stack-body">
                <Show when={termBg()}>
                    <div class="absolute inset-0 z-0 pointer-events-none" style={termBg()} />
                </Show>
                <Show when={runtimeLabel()}>
                    <div class="agent-runtime-badge" title="Agent running time">
                        {runtimeLabel()}
                    </div>
                </Show>
                {content}
            </div>
        ),
    };
}




// Register TerminalView with the ViewModel to break the circular dependency
setTerminalViewComponent(TerminalView);
// Same late-binding registration, for the hoisted chrome half — see
// setTermPaneChromeModel's own comment in termViewModel.ts.
setTermPaneChromeModel(buildTermPaneChromeModel);

export { TermViewModel };
