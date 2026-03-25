// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getFileSubject } from "@/app/store/wps";
import { sendWSCommand } from "@/app/store/ws";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { WOS, atoms, fetchWaveFile, getSettingsKeyAtom, globalStore, openLink } from "@/app/store/global";
import * as services from "@/app/store/services";
import { PLATFORM, PlatformLinux, PlatformMacOS, PlatformWindows } from "@/util/platformutil";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { base64ToArray, fireAndForget } from "@/util/util";
import { SearchAddon } from "@xterm/addon-search";
import { SerializeAddon } from "@xterm/addon-serialize";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { CanvasAddon } from "@xterm/addon-canvas";
import { UnicodeGraphemesAddon } from "@xterm/addon-unicode-graphemes";
import { WebglAddon } from "@xterm/addon-webgl";
import * as TermTypes from "@xterm/xterm";
import { Terminal } from "@xterm/xterm";
import debug from "debug";
import { debounce } from "throttle-debounce";
import { FilePathLinkProvider, makeFilePathHandler } from "./filelinkprovider";
import { FitAddon } from "./fitaddon";
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
        this.fitAddon.noScrollbar = PLATFORM === PlatformMacOS;
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
                    if (!globalStore.get(copyOnSelectAtom)) {
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
        this.fitAddon.fit();
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
    private scheduleRafWrite(data: Uint8Array) {
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
            this.rafBuffer = [];
            this.writeInFlight = true;
            this.doTerminalWrite(merged, null).then(() => {
                this.writeInFlight = false;
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
        this.fitAddon.fit();
        if (oldRows !== this.terminal.rows || oldCols !== this.terminal.cols) {
            this.sendTermSize();
        }
        dlog("resize", `${this.terminal.rows}x${this.terminal.cols}`, `${oldRows}x${oldCols}`);
    }

    // ── Private helpers ────────────────────────────────────────────────

    private loadRendererAddon(useWebGl: boolean) {
        // WebKitGTK's WebGL renderer does not correctly handle control sequences
        // (backspace \x08, erase-in-line ESC[K) — force Canvas on Linux.
        if (PLATFORM === PlatformLinux) {
            const canvasAddon = new CanvasAddon();
            this.toDispose.push(canvasAddon);
            this.terminal.loadAddon(canvasAddon);
            return;
        }
        if (WebGLSupported && useWebGl) {
            try {
                const webglAddon = new WebglAddon();
                this.toDispose.push(
                    webglAddon.onContextLoss(() => {
                        webglAddon.dispose();
                        console.warn("WebGL context lost, falling back to Canvas renderer");
                        const canvasAddon = new CanvasAddon();
                        this.toDispose.push(canvasAddon);
                        this.terminal.loadAddon(canvasAddon);
                    })
                );
                this.terminal.loadAddon(webglAddon);
                if (!loggedWebGL) {
                    console.log("loaded webgl renderer!");
                    loggedWebGL = true;
                }
            } catch (e) {
                console.warn("WebGL renderer unavailable, using Canvas renderer:", e);
                const canvasAddon = new CanvasAddon();
                this.toDispose.push(canvasAddon);
                this.terminal.loadAddon(canvasAddon);
                if (!loggedWebGL) {
                    console.log("loaded canvas renderer (webgl fallback)!");
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
