// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getFileSubject } from "@/app/store/wps";
import { sendWSCommand } from "@/app/store/ws";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { WOS, atoms, fetchWaveFile, getSettingsKeyAtom, openLink } from "@/app/store/global";
import * as services from "@/app/store/services";
import { PLATFORM, PlatformLinux, PlatformMacOS, PlatformWindows } from "@/util/platformutil";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { base64ToArray, fireAndForget } from "@/util/util";
import { SearchAddon } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { WebLinksAddon } from "@xterm/addon-web-links";

import { UnicodeGraphemesAddon } from "@xterm/addon-unicode-graphemes";
import { WebglAddon } from "@xterm/addon-webgl";
import * as TermTypes from "@xterm/xterm";
import { Terminal } from "@xterm/xterm";
import debug from "debug";
import { debounce } from "throttle-debounce";
import { FilePathLinkProvider, makeFilePathHandler } from "./filelinkprovider";
import { FitAddon } from "@xterm/addon-fit";
import { registeredAgentsByBlock, unregisterAgent } from "./termagent";
import { handleOsc7Command, handleOsc16162Command, handleOscTitleCommand, handleOscWaveCommand } from "./termosc";

const dlog = debug("wave:termwrap");

const TermFileName = "term";
const TermCacheFileName = "cache:term:full";
const MinDataProcessedForCache = 100 * 1024;

function detectWebGLSupport(): boolean {
    try {
        const canvas = document.createElement("canvas");
        const ctx = canvas.getContext("webgl");
        return !!ctx;
    } catch (e) {
        return false;
    }
}

const WebGLSupported = detectWebGLSupport();
let loggedWebGL = false;

type TermWrapOptions = {
    keydownHandler?: (e: KeyboardEvent) => boolean;
    useWebGl?: boolean;
    sendDataHandler?: (data: string) => void;
};

/**
 * TermWrap — xterm.js wrapper with a strict 3-phase lifecycle.
 *
 * Phase 1: CONSTRUCT (sync) — create Terminal, load addons, register OSC handlers.
 *          NO DOM mount, NO data subscription, NO backend communication.
 *
 * Phase 2: INIT (async) — mount to DOM, subscribe to data stream, load initial data,
 *          flush buffered data, THEN resync controller (which spawns the PTY).
 *          This ordering eliminates the race condition where PTY output arrives
 *          before the frontend is ready to receive it.
 *
 * Phase 3: RUNNING — handleResize, receive data, periodic cache.
 */
export class TermWrap {
    blockId: string;
    ptyOffset: number;
    dataBytesProcessed: number;
    terminal: Terminal;
    connectElem: HTMLDivElement;
    fitAddon: FitAddon;
    searchAddon: SearchAddon;
    serializeAddon: SerializeAddon;
    mainFileSubject: SubjectWithRef<WSFileEventData>;
    loaded: boolean;
    heldData: Uint8Array[];
    handleResize_debounced: () => void;
    hasResized: boolean;
    multiInputCallback: (data: string) => void;
    sendDataHandler: (data: string) => void;
    onSearchResultsDidChange?: (result: { resultIndex: number; resultCount: number }) => void;
    private toDispose: TermTypes.IDisposable[] = [];
    pasteActive: boolean = false;
    lastUpdated: number;
    private rafBuffer: Uint8Array[] = [];
    private rafPending: boolean = false;
    private writeInFlight: boolean = false;

    // ── Phase 1: CONSTRUCT (sync) ──────────────────────────────────────

    constructor(
        blockId: string,
        connectElem: HTMLDivElement,
        options: TermTypes.ITerminalOptions & TermTypes.ITerminalInitOnlyOptions,
        waveOptions: TermWrapOptions
    ) {
        this.loaded = false;
        this.blockId = blockId;
        this.sendDataHandler = waveOptions.sendDataHandler;
        this.ptyOffset = 0;
        this.dataBytesProcessed = 0;
        this.lastUpdated = Date.now();
        this.connectElem = connectElem;
        this.mainFileSubject = null;
        this.heldData = [];
        this.hasResized = false;
        this.handleResize_debounced = debounce(50, this.handleResize.bind(this));

        // Create terminal and load addons
        // scrollOnUserInput: false — prevents scroll-to-bottom on keystrokes, letting the user
        //   read scrollback while the PTY is active (xterm.js >= 5.1.0).
        // smoothScrollDuration: 0 — disables animated scrolling, which makes cursor-tracking
        //   viewport jumps (caused by Ink's erase-and-redraw pattern) more disorienting.
        // cursorBlink: false — disable blink by default so the xterm.js requestAnimationFrame
        //   cursor loop doesn't run on non-focused panes. Without this, every visible terminal
        //   pane runs a 60–120 Hz rAF loop solely for cursor blinking, keeping WKWebView's
        //   CoreAnimation observer firing continuously and driving sustained ~190% host CPU
        //   even when no PTY output is arriving. Focus/blur listeners re-enable blink only
        //   for the active pane.
        this.terminal = new Terminal({ ...options, cursorBlink: false, scrollOnUserInput: false, smoothScrollDuration: 0 });
        this.fitAddon = new FitAddon();
        this.serializeAddon = new SerializeAddon();
        this.searchAddon = new SearchAddon();
        this.terminal.loadAddon(this.searchAddon);
        this.terminal.loadAddon(this.fitAddon);
        this.terminal.loadAddon(this.serializeAddon);
        this.terminal.loadAddon(new UnicodeGraphemesAddon());
        this.terminal.loadAddon(
            new WebLinksAddon((e, uri) => {
                e.preventDefault();
                fireAndForget(() => openLink(uri));
            })
        );
        const getCwd = (): string | undefined => {
            try {
                const blockData = WOS.getObjectValue<Block>(WOS.makeORef("block", this.blockId));
                return blockData?.meta?.["cmd:cwd"];
            } catch {
                return undefined;
            }
        };
        this.terminal.registerLinkProvider(
            new FilePathLinkProvider(this.terminal, makeFilePathHandler(getCwd))
        );
        this.loadRendererAddon(waveOptions.useWebGl);

        // Register OSC handlers
        this.terminal.parser.registerOscHandler(9283, (data: string) => {
            return handleOscWaveCommand(data, this.blockId, this.loaded);
        });
        this.terminal.parser.registerOscHandler(7, (data: string) => {
            return handleOsc7Command(data, this.blockId, this.loaded);
        });
        this.terminal.parser.registerOscHandler(16162, (data: string) => {
            return handleOsc16162Command(data, this.blockId, this.loaded, this.terminal);
        });
        this.terminal.parser.registerOscHandler(0, (data: string) => {
            return handleOscTitleCommand(data, this.blockId, this.loaded);
        });
        this.terminal.parser.registerOscHandler(2, (data: string) => {
            return handleOscTitleCommand(data, this.blockId, this.loaded);
        });
        this.terminal.attachCustomKeyEventHandler(waveOptions.keydownHandler);

        // Tier-2 scroll fix: block macOS trackpad momentum scroll events.
        // After the user lifts their finger, the OS keeps emitting WheelEvents with small,
        // decaying deltaY values. These compound with Ink's cursor-up sequences (which move
        // the viewport) to produce "rocket scroll". Blocking events with |deltaY| < 4px
        // eliminates the feedback loop without affecting normal wheel or trackpad scrolling.
        this.terminal.attachCustomWheelEventHandler((ev: WheelEvent) => {
            if (Math.abs(ev.deltaY) < 4) return false;
            return true;
        });
    }

    // ── Phase 2: INIT (async) ──────────────────────────────────────────

    /**
     * Initialize the terminal with correct ordering to prevent race conditions.
     * Sequence: mount → subscribe → load data → flush held → resync controller.
     */
    async init() {
        // Mount terminal to DOM
        this.terminal.open(this.connectElem);

        // Enable cursor blink only while this pane is focused.  The textarea is
        // available after open(); focus/blur fire naturally as the user switches panes.
        this.terminal.textarea?.addEventListener("focus", () => {
            this.terminal.options.cursorBlink = true;
        });
        this.terminal.textarea?.addEventListener("blur", () => {
            this.terminal.options.cursorBlink = false;
        });

        this.setupPasteHandler();

        // Register input handlers
        const copyOnSelectAtom = getSettingsKeyAtom("term:copyonselect");
        this.toDispose.push(this.terminal.onData(this.handleTermData.bind(this)));
        this.toDispose.push(this.terminal.onKey(this.onKeyHandler.bind(this)));
        this.toDispose.push(
            this.terminal.onSelectionChange(
                debounce(50, () => {
                    if (!copyOnSelectAtom()) {
                        return;
                    }
                    const selectedText = this.terminal.getSelection();
                    if (selectedText.length > 0) {
                        clipboardWriteText(selectedText).catch((e) => console.log("clipboard write failed", e));
                    }
                })
            )
        );
        if (this.onSearchResultsDidChange != null) {
            this.toDispose.push(this.searchAddon.onDidChangeResults(this.onSearchResultsDidChange.bind(this)));
        }

        // Subscribe to PTY data stream BEFORE any backend communication.
        // This ensures we never miss data from the PTY.
        this.mainFileSubject = getFileSubject(this.blockId, TermFileName);
        this.mainFileSubject.subscribe(this.handleNewFileSubjectData.bind(this));

        // Load any existing terminal data (cache + main file)
        try {
            await this.loadInitialTerminalData();
        } finally {
            // Flush any data that arrived during loading, then open the gate
            this.flushHeldData();
            this.loaded = true;
        }

        // NOW fit and tell backend to start/resync the shell controller.
        // At this point we are fully subscribed and ready to receive data.
        this.customFit();
        this.sendTermSize();
        await this.resyncController("init");
        this.hasResized = true;

        this.runProcessIdleTimeout();
    }

    // ── Phase 3: RUNNING ───────────────────────────────────────────────

    dispose() {
        const agentId = registeredAgentsByBlock.get(this.blockId);
        if (agentId) {
            fireAndForget(() => unregisterAgent(agentId));
            registeredAgentsByBlock.delete(this.blockId);
        }
        this.toDispose.forEach((d) => {
            try {
                d.dispose();
            } catch (_) {}
        });
        if (this.mainFileSubject) {
            this.mainFileSubject.release();
        }
        try {
            this.terminal.dispose();
        } catch (e) {
            console.log("[termwrap] error disposing terminal:", e);
        }
    }

    handleTermData(data: string) {
        if (!this.loaded) {
            return;
        }
        if (this.pasteActive) {
            this.pasteActive = false;
            if (this.multiInputCallback) {
                this.multiInputCallback(data);
            }
        }
        this.sendDataHandler?.(data);
    }

    onKeyHandler(data: { key: string; domEvent: KeyboardEvent }) {
        if (this.multiInputCallback) {
            this.multiInputCallback(data.key);
        }
        // Scroll to bottom on printable input (letter, digit, space, punctuation).
        // scrollOnUserInput: false is kept off because it also fires on arrow keys,
        // Ctrl combos, and function keys — those shouldn't yank the viewport.
        const e = data.domEvent;
        if (data.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
            this.terminal.scrollToBottom();
        }
    }

    addFocusListener(focusFn: () => void) {
        this.terminal.textarea.addEventListener("focus", focusFn);
    }

    handleNewFileSubjectData(msg: WSFileEventData) {
        if (msg.fileop == "truncate") {
            this.terminal.clear();
            this.heldData = [];
        } else if (msg.fileop == "append") {
            const decodedData = base64ToArray(msg.data64);
            if (this.loaded) {
                this.scheduleRafWrite(decodedData);
            } else {
                this.heldData.push(decodedData);
            }
        } else {
            console.log("bad fileop for terminal", msg);
            return;
        }
    }

    // Tier-3 scroll fix: RAF-batched writes with sequential drain.
    //
    // PTY data arrives as separate WebSocket messages. Each doTerminalWrite() call
    // triggers an xterm.js viewport sync. When Ink's cursor-up chunk and content chunk
    // land in back-to-back messages, the viewport snaps up then back down — two flashes
    // per render cycle, visible on Windows 10 as the DWM compositor presents each snap
    // as a distinct frame.
    //
    // Buffering writes until the next animation frame coalesces same-cycle chunks into
    // one terminal.write() call so xterm.js only updates the viewport once, to the final
    // cursor position (back at bottom). Latency added: ≤ 16ms — imperceptible during
    // streaming output.
    //
    // writeInFlight ensures no second RAF fires while a slow write (large scrollback buffer)
    // is still in progress. Without this guard, rafPending resets before doTerminalWrite
    // resolves, letting a concurrent write race and causing the same flash at high bufLines.
    //
    // Fast path: small chunks (character echo, single completions) bypass RAF entirely.
    // The scroll-flicker pattern only occurs when Ink emits cursor-up + content as separate
    // WebSocket messages — that only happens with large multi-chunk outputs. A single
    // echoed character (1–4 bytes) is never split, so there is no flicker risk.
    // Bypassing RAF here eliminates the ≤16ms echo delay (and the writeInFlight stall)
    // that made typing feel sluggish during PTY output.
    private static readonly RAF_BYPASS_THRESHOLD = 512; // bytes

    private scheduleRafWrite(data: Uint8Array) {
        if (data.length <= TermWrap.RAF_BYPASS_THRESHOLD && this.rafBuffer.length === 0 && !this.writeInFlight) {
            // Small data with nothing queued: write directly to eliminate echo delay.
            // Only safe when rafBuffer is empty and no write is in flight, otherwise
            // the small chunk would be written out of order ahead of pending data.
            this.doTerminalWrite(data, null);
            return;
        }
        this.rafBuffer.push(data);
        this.armRaf();
    }

    // Schedule a RAF flush if one isn't already pending and no write is in flight.
    private armRaf() {
        if (this.rafPending || this.writeInFlight) return;
        this.rafPending = true;
        requestAnimationFrame(() => {
            this.rafPending = false;
            if (this.rafBuffer.length === 0) return;
            const totalLen = this.rafBuffer.reduce((n, b) => n + b.length, 0);
            const merged = new Uint8Array(totalLen);
            let offset = 0;
            for (const chunk of this.rafBuffer) {
                merged.set(chunk, offset);
                offset += chunk.length;
            }
            const chunkCount = this.rafBuffer.length;
            this.rafBuffer = [];
            this.writeInFlight = true;
            const t0 = performance.now();
            this.doTerminalWrite(merged, null).then(() => {
                this.writeInFlight = false;
                const elapsed = performance.now() - t0;
                const bufLines = this.terminal.buffer.active.length;
                if (elapsed > 8) {
                    console.warn(`[raf-write] SLOW chunks=${chunkCount} bytes=${totalLen} elapsed=${elapsed.toFixed(1)}ms bufLines=${bufLines}`);
                }
                // Drain any data that arrived while the write was in progress.
                if (this.rafBuffer.length > 0) this.armRaf();
            });
        });
    }

    doTerminalWrite(data: string | Uint8Array, setPtyOffset?: number): Promise<void> {
        let resolve: () => void = null;
        let prtn = new Promise<void>((presolve, _) => {
            resolve = presolve;
        });
        this.terminal.write(data, () => {
            if (setPtyOffset != null) {
                this.ptyOffset = setPtyOffset;
            } else {
                this.ptyOffset += data.length;
                this.dataBytesProcessed += data.length;
            }
            this.lastUpdated = Date.now();
            resolve();
        });
        return prtn;
    }

    handleResize() {
        const oldRows = this.terminal.rows;
        const oldCols = this.terminal.cols;
        this.customFit();
        if (oldRows !== this.terminal.rows || oldCols !== this.terminal.cols) {
            this.sendTermSize();
        }
        dlog("resize", `${this.terminal.rows}x${this.terminal.cols}`, `${oldRows}x${oldCols}`);
    }

    // ── Private helpers ────────────────────────────────────────────────

    // FitAddon v0.11.0 subtracts `overviewRuler.width || 14` from available width
    // whenever scrollback > 0. We don't use the overview ruler or Monaco-style scrollbar —
    // our CSS webkit scrollbar is 6px and overlaps the content. This corrects for the
    // discrepancy so the terminal fills the pane correctly.
    //
    // FITADDON_SCROLLBAR_ASSUMPTION: the width FitAddon reserves for the right-side
    // scrollbar/overview ruler when no overviewRuler is configured (hardcoded in addon-fit.js).
    // CSS_SCROLLBAR_WIDTH: our actual webkit scrollbar width (term.scss .xterm-viewport).
    // If either value changes, update both constants to match.
    private static readonly FITADDON_SCROLLBAR_ASSUMPTION = 14; // px — FitAddon's `overviewRuler.width || 14`
    private static readonly CSS_SCROLLBAR_WIDTH = 6;            // px — our webkit scrollbar (term.scss)
    private static readonly FIT_WIDTH_CORRECTION =
        TermWrap.FITADDON_SCROLLBAR_ASSUMPTION - TermWrap.CSS_SCROLLBAR_WIDTH; // = 8px

    private customFit() {
        const dims = this.fitAddon.proposeDimensions();
        if (!dims) return;
        const core = (this.terminal as any)._core;
        const cellWidth: number = core?._renderService?.dimensions?.css?.cell?.width ?? 0;
        if (cellWidth > 0) {
            dims.cols = Math.max(2, dims.cols + Math.floor(TermWrap.FIT_WIDTH_CORRECTION / cellWidth));
        }
        if (this.terminal.rows !== dims.rows || this.terminal.cols !== dims.cols) {
            core?._renderService?.clear?.();
            this.terminal.resize(dims.cols, dims.rows);
        }
    }

    private loadRendererAddon(useWebGl: boolean) {
        // WebKitGTK's WebGL2 implementation has systemic rendering issues —
        // texture atlas doesn't redraw after control sequences (backspace, erase-in-line).
        // This is a WebKitGTK bug, not xterm.js (Tauri #6559, WebKit Bug 228268).
        // Default to DOM renderer on Linux; WebGL opt-in via term:disablewebgl=false.
        if (PLATFORM === PlatformLinux && !useWebGl) {
            if (!loggedWebGL) {
                console.log("linux: using DOM renderer (WebKitGTK WebGL workaround)");
                loggedWebGL = true;
            }
            return; // DOM renderer is the default when no renderer addon is loaded
        }
        if (WebGLSupported && useWebGl) {
            try {
                const webglAddon = new WebglAddon();
                this.toDispose.push(
                    webglAddon.onContextLoss(() => {
                        webglAddon.dispose();
                        console.warn("WebGL context lost, falling back to DOM renderer");
                        // DOM renderer is active by default when no addon is loaded
                    })
                );
                this.terminal.loadAddon(webglAddon);
                if (!loggedWebGL) {
                    console.log("loaded webgl renderer!");
                    loggedWebGL = true;
                }
            } catch (e) {
                console.warn("WebGL renderer unavailable, using DOM renderer:", e);
                if (!loggedWebGL) {
                    console.log("loaded DOM renderer (webgl fallback)!");
                    loggedWebGL = true;
                }
            }
        }
    }

    private setupPasteHandler() {
        let pasteEventHandler = () => {
            this.pasteActive = true;
            setTimeout(() => {
                this.pasteActive = false;
            }, 30);
        };
        pasteEventHandler = pasteEventHandler.bind(this);
        this.connectElem.addEventListener("paste", pasteEventHandler, true);
        this.toDispose.push({
            dispose: () => {
                this.connectElem.removeEventListener("paste", pasteEventHandler, true);
            },
        });
    }

    private flushHeldData() {
        for (const data of this.heldData) {
            this.doTerminalWrite(data, null);
        }
        this.heldData = [];
    }

    private sendTermSize() {
        const termSize: TermSize = { rows: this.terminal.rows, cols: this.terminal.cols };
        const wsCommand: SetBlockTermSizeWSCommand = {
            wscommand: "setblocktermsize",
            blockid: this.blockId,
            termsize: termSize,
        };
        sendWSCommand(wsCommand);
    }

    async resyncController(reason: string) {
        dlog("resync controller", this.blockId, reason);
        const tabId = atoms.staticTabId();
        const rtOpts: RuntimeOpts = { termsize: { rows: this.terminal.rows, cols: this.terminal.cols } };
        try {
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: tabId,
                blockid: this.blockId,
                rtopts: rtOpts,
            });
        } catch (e) {
            console.log(`error controller resync (${reason})`, this.blockId, e);
        }
    }

    private async loadInitialTerminalData(): Promise<void> {
        let startTs = Date.now();
        const { data: cacheData, fileInfo: cacheFile } = await fetchWaveFile(this.blockId, TermCacheFileName);
        let ptyOffset = 0;
        if (cacheFile != null) {
            ptyOffset = cacheFile.meta["ptyoffset"] ?? 0;
            if (cacheData.byteLength > 0) {
                const curTermSize: TermSize = { rows: this.terminal.rows, cols: this.terminal.cols };
                const fileTermSize: TermSize = cacheFile.meta["termsize"];
                let didResize = false;
                if (
                    fileTermSize != null &&
                    (fileTermSize.rows != curTermSize.rows || fileTermSize.cols != curTermSize.cols)
                ) {
                    console.log("terminal restore size mismatch, temp resize", fileTermSize, curTermSize);
                    this.terminal.resize(fileTermSize.cols, fileTermSize.rows);
                    didResize = true;
                }
                this.doTerminalWrite(cacheData, ptyOffset);
                if (didResize) {
                    this.terminal.resize(curTermSize.cols, curTermSize.rows);
                }
            }
        }
        const { data: mainData, fileInfo: mainFile } = await fetchWaveFile(this.blockId, TermFileName, ptyOffset);
        console.log(
            `terminal loaded cachefile:${cacheData?.byteLength ?? 0} main:${mainData?.byteLength ?? 0} bytes, ${Date.now() - startTs}ms`
        );
        if (mainFile != null) {
            await this.doTerminalWrite(mainData, null);
        }
    }

    processAndCacheData() {
        if (this.dataBytesProcessed < MinDataProcessedForCache) {
            return;
        }
        const serializedOutput = this.serializeAddon.serialize();
        const termSize: TermSize = { rows: this.terminal.rows, cols: this.terminal.cols };
        console.log("idle timeout term", this.dataBytesProcessed, serializedOutput.length, termSize);
        fireAndForget(() =>
            services.BlockService.SaveTerminalState(this.blockId, serializedOutput, "full", this.ptyOffset, termSize)
        );
        this.dataBytesProcessed = 0;
    }

    private runProcessIdleTimeout() {
        setTimeout(() => {
            if (typeof window.requestIdleCallback === "function") {
                window.requestIdleCallback(() => {
                    this.processAndCacheData();
                    this.runProcessIdleTimeout();
                });
            } else {
                this.processAndCacheData();
                this.runProcessIdleTimeout();
            }
        }, 5000);
    }
}
