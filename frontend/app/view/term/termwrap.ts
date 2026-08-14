// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getFileSubject } from "@/app/store/wps";
import { sendWSCommand } from "@/app/store/ws";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS, atoms, fetchWaveFile, getSettingsKeyAtom, openLink, setTermRendererAtom } from "@/app/store/global";
import * as services from "@/app/store/services";
import { PLATFORM, PlatformMacOS, PlatformWindows } from "@/util/platformutil";
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
import { markStart, markEnd } from "@/perf";
import { PredictiveEcho } from "./predictive-echo";

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
    // Chunks buffered while !loaded (subscribed but the initial fetch hasn't
    // resolved yet). `offset` is the chunk's start position in the server-side
    // file, when known (see WSFileEventData.offset) — used by flushHeldData to
    // reconcile against what loadInitialTerminalData's fetch already covered,
    // so a chunk landing in that window isn't rendered (and counted into
    // ptyOffset) twice. See SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1.
    heldData: { data: Uint8Array; offset?: number }[];
    handleResize_debounced: () => void;
    hasResized: boolean;
    multiInputCallback: (data: string) => void;
    sendDataHandler: (data: string) => void;
    onSearchResultsDidChange?: (result: { resultIndex: number; resultCount: number }) => void;
    private toDispose: TermTypes.IDisposable[] = [];
    pasteActive: boolean = false;
    lastUpdated: number;
    // Thaw-cycle handles — cleared in dispose() so callbacks don't fire
    // on a disposed Terminal. dispose() doesn't null `this.terminal`, so
    // a null-check guard alone isn't enough.
    private thawTimeoutId: ReturnType<typeof setTimeout> | null = null;
    private thawRafId: number | null = null;
    private disposed: boolean = false;
    // Tracks the fire-and-forget resyncController("init") call so the
    // post-paint rAF re-fit (below) can wait for the controller to actually
    // exist before sending a corrective resize, instead of racing it — see
    // the comment at the rAF callback for why.
    private initResyncPromise: Promise<void> = Promise.resolve();

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
        // Reverse-wraparound (DECSET ?45): Backspace at column 0 of a soft-wrapped
        // line moves to the END of the previous row, matching real terminals.
        // bash/readline emits a bare BS (0x08) for the cross-wrap delete and relies
        // on the terminal honoring reverse-wrap (xterm-256color has no `bw`). xterm.js
        // defaults this mode OFF — so on a raw PTY (Linux/macOS) backspacing across a
        // wrap is stuck at col 0. (Windows is unaffected: ConPTY re-renders with
        // absolute cursor positioning, so the bare BS never reaches xterm.js.) The
        // enabled branch only wraps when the row is genuinely `isWrapped`, so hard
        // line breaks are never crossed. We assert it here AND re-assert on RIS reset.
        this.terminal.write("\x1b[?45h");
        // A full reset (RIS, `ESC c` — e.g. `reset` / `tput reset`) restores DEC
        // private modes to defaults, turning reverse-wrap back OFF. Re-assert it
        // immediately after, so a reset doesn't silently reintroduce the stuck-
        // backspace bug. Returning false lets xterm run its built-in RIS first;
        // the microtask then re-applies the mode once that synchronous parse ends.
        this.terminal.parser.registerEscHandler({ final: "c" }, () => {
            queueMicrotask(() => this.terminal.write("\x1b[?45h"));
            return false;
        });
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
            if (ev.ctrlKey) return false;           // propagate for Ctrl+Wheel zoom handling
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

        // Predictive local echo (spec §6). Sink writes directly to xterm: paint
        // appends a glyph; erase walks the cursor back N cells and clears to EOL
        // (Phase-1 appended-at-cursor model). Gated on term:predictiveecho.
        const predictiveEchoAtom = getSettingsKeyAtom("term:predictiveecho");
        const predictiveThreshold = getSettingsKeyAtom("term:predictiveecho:thresholdms")();
        this.predict = new PredictiveEcho(
            {
                paint: (g) => this.terminal.write(g),
                erase: (n) => this.terminal.write(`\x1b[${n}D\x1b[K`),
            },
            {
                // Enabled by DEFAULT — opt out via term:predictiveecho=false.
                enabled: () => predictiveEchoAtom() !== false,
                // Predict whenever armed by default; set term:predictiveecho:thresholdms
                // to re-enable the RTT gate (0 = always-on once a confirmed echo arms).
                thresholdMs: typeof predictiveThreshold === "number" ? predictiveThreshold : 0,
            },
        );

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

        // Force-load the configured term font BEFORE the first fit, with a bounded
        // timeout. proposeDimensions() measures rendered cell width from the DOM; if
        // the configured term font (e.g. Hack) hasn't loaded yet, FitAddon uses fallback
        // metrics and computes wrong cols, the PTY is told the wrong size, and TUIs
        // (Ink/Claude Code) emit cursor sequences that don't line up — visible as
        // jumbled glyphs until the next resize.
        //
        // Why `document.fonts.load(spec)` and not `document.fonts.ready`:
        //   - `fonts.ready` only awaits font faces that are ALREADY pending. Web fonts
        //     are loaded lazily — the browser doesn't request the term font's WOFF/WOFF2
        //     until something actually renders a glyph in that family. `terminal.open()`
        //     above mounts the DOM but the font request typically isn't initiated until
        //     `proposeDimensions()` measures a cell. So `await fonts.ready` resolves
        //     vacuously (no pending loads) and `customFit()` then measures with fallback
        //     metrics. This was the hole left by PR #1030's first cut — the same jumbled
        //     glyphs reproduced when opening multiple terminals fast.
        //   - `fonts.load("12px Hack")` actively REQUESTS the font and resolves only
        //     when that specific face is ready. Subsequent terminals coalesce on the
        //     same browser-level cache entry, so opening N panes in parallel waits
        //     once.
        //
        // The timeout is essential: a stalled font load would block
        // sendTermSize()/resyncController("init") and the pane's PTY would never start
        // (`frontend/app-init.ts` wraps the app-level wait in a 2s race for the same
        // reason). The rAF re-fit below catches the case where the font lands just
        // after the timeout.
        //
        // We load the regular + bold + italic variants xterm.js uses, in parallel.
        // Browsers dedupe identical font face requests, so this costs ~one network
        // round-trip total on cold cache.
        const FIT_FONT_TIMEOUT_MS = 1000;
        const fontFamily = this.terminal.options.fontFamily ?? "Hack";
        const fontSize = this.terminal.options.fontSize ?? 12;
        const fontSpec = (variant: string) => `${variant}${fontSize}px "${fontFamily}"`;
        try {
            await Promise.race([
                Promise.all([
                    document.fonts?.load(fontSpec("")) ?? Promise.resolve(),
                    document.fonts?.load(fontSpec("bold ")) ?? Promise.resolve(),
                    document.fonts?.load(fontSpec("italic ")) ?? Promise.resolve(),
                ]),
                new Promise<void>((resolve) => setTimeout(resolve, FIT_FONT_TIMEOUT_MS)),
            ]);
        } catch (_) { /* font API unavailable or face unknown — fall through */ }

        // NOW fit and tell backend to start/resync the shell controller.
        // At this point we are fully subscribed and ready to receive data.
        this.customFit();
        this.sendTermSize();
        // hasResized must flip before the fire-and-forget resync below: it gates the
        // TermResyncHandler effect, and the terminal is already sized at this point
        // (customFit/sendTermSize above), so a reconnect event arriving mid-spawn must
        // not be dropped. See issue #121.
        this.hasResized = true;
        // fire-and-forget — PTY data subscription is already active. Kept so the rAF
        // re-fit below can wait for the controller to exist before resizing it: the
        // backend drops setblocktermsize entirely (no queue/retry) when the block's
        // controller hasn't been created yet, and resyncController("init") is what
        // creates it.
        this.initResyncPromise = this.resyncController("init");

        // One re-fit after first paint to catch any remaining layout shift (slow CSS,
        // late style recalculation, font swap that landed after fonts.ready resolved).
        // If dimensions changed, sendTermSize() issues a SIGWINCH to the PTY so the
        // controller redraws against the correct size before producing meaningful output.
        // Waits on initResyncPromise first (only when a resize is actually needed —
        // the common no-change case stays instant): otherwise this can race ahead of
        // the still-in-flight "init" resync and get silently dropped for having no
        // controller yet, leaving the PTY at stale geometry until a manual resize.
        requestAnimationFrame(() => {
            if (!this.terminal) return;
            const oldRows = this.terminal.rows;
            const oldCols = this.terminal.cols;
            this.customFit();
            if (oldRows !== this.terminal.rows || oldCols !== this.terminal.cols) {
                void this.initResyncPromise.then(() => this.sendTermSize());
            }
        });

        // PSReadLine cursor-desync "thaw" — see #1042 / docs/analysis/archive/
        // TERM_JUMBLE_STRUCTURED_2026_05_25.md §7a.
        //
        // When a terminal is created without subsequent sibling-pane
        // splits, its only init-time resize is the default-80 → final
        // transition. pwsh's PSReadLine emits its first prompt against
        // the inherited (cols=80) ConPTY environment, then xterm's
        // SIGWINCH from sendTermSize lands as the final cols (e.g. 14).
        // PSReadLine's tracked cursor diverges from xterm's actual
        // cursor — visible as "cursor jumps on Enter," cursor desync,
        // prompt mis-layout. Manual pane resize fixes it because the
        // resulting SIGWINCH triggers PSReadLine to re-sync.
        //
        // Terminals that DID get subsequent resizes (because later
        // panes shrank them) accumulated several corrective SIGWINCH
        // events that re-synced PSReadLine. To get the same outcome
        // unconditionally, replay one synthetic resize cycle ~250ms
        // post-init: that gap is enough for the first prompt to land
        // but short enough to feel immediate.
        //
        // Sends the two SIGWINCH-equivalent notifications directly via
        // sendTermSize(override) WITHOUT ever calling this.terminal.resize() —
        // PSReadLine's resync only needs the BACKEND PTY to see a size change;
        // the frontend's own rendered grid never needs to move at all to
        // achieve that. The original implementation called
        // this.terminal.resize() for both steps, which is real and visible:
        // xterm reallocates its canvas and redraws at the new grid size, a
        // one-frame ~9px width blip confirmed via live CDP trace (every open,
        // ~300-500ms after mount) — see
        // docs/specs/SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md.
        // Still split across rAF ticks (not just two sendTermSize calls back
        // to back) to preserve the original timing shape the backend/PSReadLine
        // side was tuned against, even though xterm itself no longer coalesces
        // anything here (there's no xterm-side resize to coalesce).
        //
        // Gated to Windows (PLATFORM === "win32") because PSReadLine /
        // ConPTY are the affected stack. Non-Windows shells (bash, zsh,
        // fish) handle SIGWINCH cleanly on their own and the extra
        // resize cycle would be needless overhead. Reagent P2.
        //
        // Handles tracked on `this` and cleared in dispose() so the
        // callbacks can't fire on a disposed Terminal. Reagent P1.
        if (PLATFORM === PlatformWindows) {
            this.thawTimeoutId = setTimeout(() => {
                this.thawTimeoutId = null;
                if (this.disposed || !this.terminal) return;
                const baseCols = this.terminal.cols;
                const baseRows = this.terminal.rows;
                if (baseCols < 4) return; // too narrow to safely toggle ±1
                try {
                    this.sendTermSize({ cols: baseCols + 1, rows: baseRows });
                    this.thawRafId = requestAnimationFrame(() => {
                        this.thawRafId = null;
                        if (this.disposed || !this.terminal) return;
                        // If a REAL resize happened between step 1 and now (e.g. a
                        // sibling-split firing handleResize), the terminal's actual
                        // grid has already moved on and already sent its own correct
                        // SIGWINCH — sending our stale baseCols/baseRows now would
                        // clobber that with an outdated size. Unlike the previous
                        // (resize-based) version, step 1 never touched
                        // this.terminal.cols/rows at all, so comparing against
                        // baseCols/baseRows directly (not some step-1-mutated target)
                        // is what detects "did anything real happen meanwhile."
                        // Codex P2 on #1043 (same underlying concern, adapted).
                        if (this.terminal.cols !== baseCols || this.terminal.rows !== baseRows) {
                            return;
                        }
                        try {
                            this.sendTermSize({ cols: baseCols, rows: baseRows });
                        } catch (e) {
                            console.warn("[term] PSReadLine-thaw step 2 failed", e);
                        }
                    });
                } catch (e) {
                    console.warn("[term] PSReadLine-thaw step 1 failed", e);
                }
            }, 250);
        }

        this.runProcessIdleTimeout();
    }

    // ── Phase 3: RUNNING ───────────────────────────────────────────────

    dispose() {
        this.disposed = true;
        // Cancel pending PSReadLine-thaw callbacks so they don't fire
        // on a disposed Terminal. dispose() doesn't null `this.terminal`
        // (xterm.Terminal stays in memory until its own dispose finishes
        // and references are dropped), so the `if (!this.terminal)`
        // guards in the callbacks can't catch this race on their own.
        if (this.thawTimeoutId !== null) {
            clearTimeout(this.thawTimeoutId);
            this.thawTimeoutId = null;
        }
        if (this.thawRafId !== null) {
            cancelAnimationFrame(this.thawRafId);
            this.thawRafId = null;
        }
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

    // Predictive local echo: paints a just-typed printable char in the keydown
    // frame, reconciled against the authoritative PTY echo. Null until init().
    // Spec: docs/specs/SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md.
    private predict: PredictiveEcho | null = null;

    handleTermData(data: string) {
        if (!this.loaded) {
            return;
        }
        markStart('term-keypress');
        if (this.pasteActive) {
            this.pasteActive = false;
            if (this.multiInputCallback) {
                this.multiInputCallback(data);
            }
        }
        this.sendDataHandler?.(data);
        // Flush predictions when the cursor is at or past the last terminal
        // column. The rollback sequence (CSI n D + CSI K) only works within
        // the current row — any painted glyph that would wrap onto the next
        // line can't be reliably erased, violating the convergence invariant.
        // Resetting here keeps predictions within the safe single-row window;
        // re-arming happens normally on the next confirmed echo after the wrap.
        // (reagent P2 / codex P1 on #1223.)
        if (this.predict?.isArmed || this.predict?.pending) {
            const cx = this.terminal?.buffer?.active?.cursorX ?? 0;
            const cols = this.terminal?.cols ?? 80;
            if (cx >= cols - 1) {
                this.predict.reset();
            }
        }
        // Optimistically paint the keystroke (no-op unless term:predictiveecho is
        // on AND the echo round-trip is above threshold AND we're armed).
        this.predict?.onInput(data);
        markEnd('term-keypress', 'sent');
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
                // Write straight to xterm.js — its built-in RenderDebouncer is the
                // single frame-coalescer (one render per animation frame). We no
                // longer run our own RAF coalescer on top (the "double rAF" removed
                // in SPEC_TERM_DOUBLE_RAF_TEAROUT). The ≤32B echo perf mark is
                // started here and closed in doTerminalWrite's write callback.
                if (decodedData.length <= 32) markStart('term-echo-render');
                this.doTerminalWrite(decodedData, null);
            } else {
                this.heldData.push({ data: decodedData, offset: msg.offset });
            }
        } else {
            console.log("bad fileop for terminal", msg);
            return;
        }
    }

    // PTY output is written straight to xterm.js (doTerminalWrite). xterm's own
    // RenderDebouncer is the single frame-coalescer — one render per animation
    // frame, with dirty rows merged. We previously ran our OWN RAF write-coalescer
    // on top of that (Stage-1 rAF: scheduleRafWrite/armRaf), a second frame gate
    // that beat against xterm's rAF and caused uneven typing frame-pacing. Removed
    // in SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30.
    //
    // The Stage-1 rAF originally fixed a Windows-10 DWM scroll-flash with Ink TUIs
    // (cursor-up + content arriving as separate WS messages presented as two
    // frames; PR #208) — invisible on Windows 11, where the compositor coalesces
    // within vsync. xterm's debouncer renders only once per frame, so the
    // intermediate up/down viewport state should not paint separately even on
    // Win10; that must be re-verified on real Windows 10 (see the spec). If the
    // flash regresses, the fix is backend-side read coalescing (merge consecutive
    // PTY reads into one WS message in agentmux-srv), NOT a second frontend rAF.
    doTerminalWrite(data: string | Uint8Array, setPtyOffset?: number): Promise<void> {
        const originalLen = data.length;
        const isSmall = originalLen <= 32;
        // When predictive echo is on, reconcile this authoritative chunk against
        // outstanding predictions (spec §6.3): consume already-painted echoes,
        // roll back on divergence, write only what the user can't already see.
        // ptyOffset still advances by the FULL chunk — these are real PTY bytes.
        let resolve: () => void = null;
        let prtn = new Promise<void>((presolve, _) => {
            resolve = presolve;
        });
        const settle = () => {
            if (setPtyOffset != null) {
                this.ptyOffset = setPtyOffset;
            } else {
                this.ptyOffset += originalLen;
                this.dataBytesProcessed += originalLen;
            }
            this.lastUpdated = Date.now();
            if (isSmall) markEnd('term-echo-render', 'rendered');
            resolve();
        };

        if (this.predict && data instanceof Uint8Array) {
            if (this.predict.isEnabled()) {
                // Skip reconcile entirely when fully dormant (reagent #1223 P2).
                if (this.predict.pending > 0 || this.predict.isArmed) {
                    // reconcile operates on raw bytes — the queue only holds single
                    // ASCII chars, so byte comparison is exact. `rest` is the
                    // unconsumed tail returned as the ORIGINAL Uint8Array so
                    // multibyte UTF-8 sequences split across WS chunks are NEVER
                    // decoded, preserving all non-ASCII output (codex P2 on #1223).
                    const { auth, rest } = this.predict.reconcile(data);
                    this.predict.sweep();
                    const hasAuth = auth.length > 0;
                    const hasRest = rest.length > 0;
                    if (!hasAuth && !hasRest) {
                        settle(); // fully consumed by prediction
                    } else if (hasAuth && !hasRest) {
                        this.terminal.write(auth, settle);
                    } else if (!hasAuth && hasRest) {
                        this.terminal.write(rest, settle);
                    } else {
                        // Write observations first, then the unconsumed tail.
                        // settle fires after the second write.
                        this.terminal.write(auth, () => this.terminal.write(rest, settle));
                    }
                    return prtn;
                }
            } else if (this.predict.pending > 0) {
                this.predict.reset(); // roll back on disable (codex #1223 P2)
            }
        }
        // Default / pass-through path.
        this.terminal.write(data, settle);
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

    private customFit() {
        const dims = this.fitAddon.proposeDimensions();
        // proposeDimensions can return {cols: NaN, rows: NaN} when the DOM cell measurement
        // fails (font not loaded, hidden container, zero pixel dimensions). The truthy check
        // alone doesn't catch this because the object exists — NaN < N is always false,
        // so it propagates through to terminal.resize() and corrupts the rendered grid.
        if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return;
        const core = (this.terminal as any)._core;
        const cellWidth: number = core?._renderService?.dimensions?.css?.cell?.width ?? 0;
        if (cellWidth > 0 && this.connectElem) {
            // FitAddon unconditionally subtracts a 14px scrollbar gutter from the available
            // width (`scrollback > 0 ? overviewRuler.width || 14 : 0`), leaving a permanent
            // dead lane to the right of the text
            // (docs/analysis/ANALYSIS_TERM_SCROLLBAR_GAP_ROOTCAUSE_2026_06_10.md). xterm 6's
            // scrollbar is a transform-driven OVERLAY (.xterm-scrollable-element > .scrollbar)
            // that floats over the content and reserves no layout, so that gutter is pure
            // waste. Recompute cols from the full container width to reclaim it. The overlay
            // scrollbar fades in over the rightmost column when there's scrollback and is
            // lifted above the link layer in term.scss so it stays clickable.
            const cs = getComputedStyle(this.connectElem); // perf:allow-layout-read — customFit runs on resize/init/refit, never the keystroke path
            const padX = (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0);
            const availPx = this.connectElem.clientWidth - padX; // perf:allow-layout-read — resize/fit path, not per-keystroke
            dims.cols = Math.max(2, Math.floor(availPx / cellWidth));
        }
        if (this.terminal.rows !== dims.rows || this.terminal.cols !== dims.cols) {
            core?._renderService?.clear?.();
            this.terminal.resize(dims.cols, dims.rows);
        }
    }

    private loadRendererAddon(useWebGl: boolean) {
        // Linux used to default away from WebGL here (WebKitGTK-era workaround, back
        // when the Linux build ran on a WebKitGTK webview). That's moot post-CEF —
        // there is no WebKitGTK in this app anymore — and the real Linux GPU-backend
        // problem it was papering over (ANGLE defaulting to software Vulkan on VMware
        // guests) is now fixed properly at the host level via capability-probed ANGLE
        // backend selection (PR #1394, docs/specs/SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md).
        // WebGL is now the default on every platform; term:disablewebgl remains the
        // manual escape hatch (handled by the `else` branch below on any platform).
        if (WebGLSupported && useWebGl) {
            try {
                const webglAddon = new WebglAddon();
                this.toDispose.push(
                    webglAddon.onContextLoss(() => {
                        webglAddon.dispose();
                        console.warn("WebGL context lost, falling back to DOM renderer");
                        setTermRendererAtom("dom"); // GPU context lost → DOM fallback
                        // DOM renderer is active by default when no addon is loaded
                    })
                );
                this.terminal.loadAddon(webglAddon);
                setTermRendererAtom("webgl");
                if (!loggedWebGL) {
                    console.log("loaded webgl renderer!");
                    loggedWebGL = true;
                }
            } catch (e) {
                console.warn("WebGL renderer unavailable, using DOM renderer:", e);
                setTermRendererAtom("dom");
                if (!loggedWebGL) {
                    console.log("loaded DOM renderer (webgl fallback)!");
                    loggedWebGL = true;
                }
            }
        } else {
            // WebGL unsupported (GPU disabled) or disabled by setting — DOM renderer.
            setTermRendererAtom("dom");
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
        // this.ptyOffset is a fixed snapshot here (set by loadInitialTerminalData,
        // which already awaited/settled before flushHeldData is called) — the
        // server-side file size as of that fetch. Each held chunk's `offset` is
        // its own start position in that same file, so comparing every chunk
        // against this single boundary is correct regardless of processing
        // order: a chunk fully below the boundary was already delivered by the
        // fetch and must be dropped (not re-rendered, not double-counted into
        // ptyOffset); a chunk straddling it needs its covered prefix trimmed.
        const fetchBoundary = this.ptyOffset;
        for (const { data, offset } of this.heldData) {
            if (offset == null) {
                // No offset info (older server, or a non-write-through event) —
                // fall back to the pre-existing always-write behavior.
                this.doTerminalWrite(data, null);
                continue;
            }
            const chunkEnd = offset + data.length;
            if (chunkEnd <= fetchBoundary) {
                continue; // fully covered by the initial fetch; skip
            } else if (offset < fetchBoundary) {
                this.doTerminalWrite(data.subarray(fetchBoundary - offset), null);
            } else {
                this.doTerminalWrite(data, null);
            }
        }
        this.heldData = [];
    }

    /**
     * `override`, when given, tells the backend PTY a size WITHOUT touching this
     * terminal's own rendered grid — used by the PSReadLine thaw below to deliver a
     * synthetic SIGWINCH pair with no visible resize (SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md).
     * Every other call site omits it and keeps sending the terminal's actual current size.
     */
    private sendTermSize(override?: TermSize) {
        const termSize: TermSize = override ?? { rows: this.terminal.rows, cols: this.terminal.cols };
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
