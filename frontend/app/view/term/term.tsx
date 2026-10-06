// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Search, useSearch } from "@/app/element/search";
import { atoms, getOverrideConfigAtom, getSettingsKeyAtom, getSettingsPrefixAtom, useBlockAtom, MOS } from "@/store/global";
import { backendStatusAtom } from "@/store/backendStatus";
import { fireAndForget } from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/muxutil";
import { ISearchOptions } from "@xterm/addon-search";
import clsx from "clsx";
import { createEffect, createMemo, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { resolveTermFontFamily } from "./termfontfamily";
import { resolveTermScrollback } from "./termscrollback";
import { TermStickers } from "./termsticker";
import { TermThemeUpdater } from "./termtheme";
import { computeTheme } from "./termutil";
import { TermViewModel } from "./termViewModel";
import { termPaneTab } from "./term-pane-tab";
import { TermWrap } from "./termwrap";
import { termModels } from "./term-models";
import "./xterm.css";
import { registerFileDropTarget } from "@/app/drag/file-drop";
import { copyIntoWorkdir, fileCount, paneWorkdir } from "@/app/drag/file-drop-actions";
import { focusManager } from "@/app/store/focusManager";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { NodeModel } from "@/layout/index";
import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
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

function TerminalView(props: { model: TermViewModel }): JSX.Element {
    const model = props.model;
    const blockId = model.blockId;
    let viewRef!: HTMLDivElement;
    let connectElemRef!: HTMLDivElement;

    // The block's meta comes from the host context (Pane Tab contract Phase 2c).
    const meta = model.meta;

    const termSettingsAtom = getSettingsPrefixAtom("term");
    const termSettings = createMemo(() => termSettingsAtom());
    const termMode = createMemo(() => meta()?.["term:mode"] ?? "term");
    const termFontSize = createMemo(() => model.fontSizeAtom());
    const termScrollSensitivity = createMemo(() => model.scrollSensitivityAtom());
    // Settings resolved once at TermWrap construction and never revisited
    // until now — see SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md
    // §6.1. Promoted from one-shot local `const`s inside onMount (below) to
    // top-level memos so onMount and the live-apply effects further down
    // share one source of truth instead of onMount re-deriving its own copy.
    const termFontFamily = createMemo(() => {
        const connFontFamily = (atoms.fullConfigAtom() as any)?.connections?.[meta()?.connection]?.[
            "term:fontfamily"
        ];
        return resolveTermFontFamily(termSettings(), connFontFamily);
    });
    const termScrollbackDepth = createMemo(() => resolveTermScrollback(termSettings(), meta()));
    // Default ON: modern shells (bash 4+, zsh, fish) all support BPM and it
    // prevents the shell from executing partial lines mid-paste. Disable
    // per-pane via term:allowbracketedpaste=false for legacy shells that
    // don't support it.
    const termAllowBracketedPaste = createMemo(() => getOverrideConfigAtom(blockId, "term:allowbracketedpaste")() ?? true);
    const isFocused = createMemo(() => model.isFocused());
    const isMI = createMemo(() => atoms.isTermMultiInput());
    const isBasicTerm = createMemo(() => meta()?.controller != "cmd");

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
            termWrap.handleResizeLive();
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
            void model.setMeta({ "term:zoom": next === 1.0 ? null : next });
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

    // File drops: this pane's hook for the window-level file-drop controller
    // (app/drag/file-drop.ts), which hit-tests, draws the indicator and
    // dispatches. A drop copies the files into the terminal's working folder
    // (SPEC_PANE_FILE_DROP_2026_05_30.md, SPEC_DRAG_AND_DROP_CONSOLIDATION §5.3).
    // Without host paths the files' bytes are copied instead.
    onMount(() => {
        const dispose = registerFileDropTarget(blockId, {
            accept(drag) {
                if (!dndEnabled()) return { ok: false, reason: "File drop is turned off (dnd:enabled)" };
                const cwd = paneWorkdir(blockId);
                const what = drag.count > 0 ? fileCount(drag.count) : "files";
                return cwd
                    ? { ok: true, message: `Copy ${what} to ${cwd}`, icon: "fa-copy" }
                    : { ok: false, reason: "No working directory for this terminal" };
            },
            async drop({ paths, files }) {
                await copyIntoWorkdir(blockId, paths.length > 0 ? { paths } : { files }, {
                    paneKind: "terminal pane",
                    concurrency: dndConcurrency(),
                });
            },
        });
        onCleanup(dispose);
    });

    return (
        <div
            ref={viewRef!}
            class={clsx("view-term", "term-mode-" + termMode())}
            style={{ position: "relative" }}
        >
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
    // background is per-block meta, the runtime label is the active
    // terminal's own model, found by block id (term-models.ts) — the host's
    // view model is only an adapter once the terminal is a native pane tab.
    const termBg = createMemo(() => computeBgStyleFromMeta(activeBlockData()?.meta, null));
    const runtimeLabel = () => termModels.get(activeBlockId())?.agentRuntimeLabel() ?? null;

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
            const meta = activeBlockData()?.meta;
            const cwd = meta?.["cmd:cwd"] as string | undefined;
            // And its connection: a WSL tab's cwd is a path inside the distro,
            // which means nothing to a local shell.
            const connection = meta?.connection as string | undefined;
            // The same key `build_pane_meta`'s own "term" branch writes from
            // a top-level `cwd` arg (pane.rs) — set directly here because
            // this path always supplies `meta`, so that branch never runs.
            const out: Record<string, string> = {};
            if (cwd) out["cmd:cwd"] = cwd;
            if (connection && connection !== "local") out.connection = connection;
            return Object.keys(out).length > 0 ? out : undefined;
        },
        rootClass: "term-pane-stack",
        contentClass: "term-pane-stack-content",
        // The background image and runtime badge span the strip AND the
        // terminal, so they need a wrapper around the content region
        // rather than a slot beside it (term.scss's own
        // `> .term-pane-stack-body` positioning depends on this box).
        bodyClass: "term-pane-stack-body",
        renderBehindContent: () => (
            <>
            <Show when={termBg()}>
                <div class="absolute inset-0 z-0 pointer-events-none" style={termBg()} />
            </Show>
            <Show when={runtimeLabel()}>
                <div class="agent-runtime-badge" title="Agent running time">
                    {runtimeLabel()}
                </div>
            </Show>
            </>
        ),
    };
}




/** The terminal as a native pane tab (Pane Tab contract Phase 2c). */
export const terminalPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "term",
    label: "Terminal",
    icon: "terminal",
    defaultHue: 240,
    capabilities: {
        // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1
        // §5): remounting loses the scrollback and the running shell's view.
        lifecycle: "keepAlive",
        headerMic: { title: "Speak into this terminal" },
        statsBadgeSetting: "term:showstatsbadge",
        hueBorder: true,
        paneZoom: {},
        acceptsInput: true,
        shellKeys: true,
        sharesCwd: true,
        connection: true,
        noPadding: true,
    },
    tab: termPaneTab,
    chrome: buildTermPaneChromeModel,
    create: (ctx) => {
        const model = new TermViewModel(ctx);
        return {
            component: () => <TerminalView model={model} />,
            liveTitle: () => ({ text: model.viewName() }),
            headerText: () => model.viewText(),
            headerActions: () => model.endIconButtons(),
            background: () => model.blockBg(),
            // A terminal running a command (`controller: "cmd"`) has no
            // connection button.
            manageConnection: () => model.manageConnection(),
            voice: () => model.voiceHandle(),
            search: () => model.searchAtoms,
            selection: () => model.getSelection(),
            paste: (text) => model.paste(text),
            focus: () => model.giveFocus(),
            dispose: () => model.dispose(),
        };
    },
};

export { TermViewModel };
