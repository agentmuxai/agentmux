// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { sleep } from "@/util/async";
import { BlockNodeModel } from "@/app/block/blocktypes";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { appHandleKeyDown } from "@/app/store/keymodel";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { sendWSCommand } from "@/app/store/ws";
import { makeFeBlockRouteId } from "@/app/store/rpc-router";
import { DefaultRouter, TabRpcClient } from "@/app/store/rpc-util";
import { TermRpcClient } from "@/app/view/term/term-rpc";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";
import {
    atoms,
    getAllBlockComponentModels,
    getBlockMetaKeyAtom,
    getConnStatusAtom,
    getOverrideConfigAtom,
    getSettingsKeyAtom,
    pushNotification,
    removeNotificationById,
    setIsTermMultiInput,
    useBlockAtom,
    WOS,
} from "@/store/global";
import * as services from "@/store/services";
import * as keyutil from "@/util/keyutil";
import { boundNumber, createSignalAtom, stringToBase64 } from "@/util/util";
import type { SignalAtom } from "@/util/util";
import { createMemo, createSignal } from "solid-js";

// Ticks every 60 s so agentRuntimeLabel memos re-evaluate without waiting for a status event.
// globalThis survives HMR module re-evaluation — prevents duplicate interval leak.
const [nowMinute, setNowMinute] = createSignal(Math.floor(Date.now() / 60_000));
if ((globalThis as any).__nowMinuteInterval != null) clearInterval((globalThis as any).__nowMinuteInterval);
(globalThis as any).__nowMinuteInterval = setInterval(() => setNowMinute(Math.floor(Date.now() / 60_000)), 60_000);
import { computeTheme, DefaultTermTheme } from "./termutil";
import { TermWrap } from "./termwrap";
import { buildSettingsMenuItems } from "./termSettingsMenu";

let _terminalViewComponent: ViewComponent = null;

export function setTerminalViewComponent(component: ViewComponent) {
    _terminalViewComponent = component;
}

class TermViewModel implements ViewModel {
    viewType: string;
    nodeModel: BlockNodeModel;
    connected: boolean;
    termRef: { current: TermWrap | null } = { current: null };
    blockAtom: () => Block;
    termMode: () => string;
    blockId: string;
    viewIcon: () => string;
    viewName: () => string;
    viewText: () => HeaderElem[];
    blockBg: () => MetaType;
    manageConnection: () => boolean;
    connStatus: () => ConnStatus;
    termRpcClient: TermRpcClient;
    fontSizeAtom: () => number;
    termZoomAtom: () => number;
    termThemeNameAtom: () => string;
    termTransparencyAtom: () => number;
    noPadding: SignalAtom<boolean>;
    endIconButtons: () => IconButtonDecl[];
    shellProcFullStatus: SignalAtom<BlockControllerRuntimeStatus>;
    shellProcStatus: () => string;
    shellProcStatusUnsubFn: () => void;
    isCmdController: () => boolean;
    isRestarting: SignalAtom<boolean>;
    agentRuntimeLabel: () => string | null;
    searchAtoms?: SearchAtoms;
    voiceHandle: () => PaneVoiceHandle;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.viewType = "term";
        this.blockId = blockId;
        this.termRpcClient = new TermRpcClient(blockId, this);
        DefaultRouter.registerRoute(makeFeBlockRouteId(blockId), this.termRpcClient);
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);

        this.termMode = createMemo(() => {
            const blockData = this.blockAtom();
            return blockData?.meta?.["term:mode"] ?? "term";
        });

        this.isRestarting = createSignalAtom(false);

        this.viewIcon = createMemo(() => "terminal");

        this.viewName = createMemo(() => {
            const blockData = this.blockAtom();
            if (blockData?.meta?.controller == "cmd") return "";
            return "Terminal";
        });

        this.isCmdController = createMemo(() => {
            const controllerMetaAtom = getBlockMetaKeyAtom(this.blockId, "controller");
            return controllerMetaAtom() == "cmd";
        });

        this.shellProcFullStatus = createSignalAtom<BlockControllerRuntimeStatus>(null);

        this.shellProcStatus = createMemo(() => {
            const fullStatus = this.shellProcFullStatus();
            return fullStatus?.shellprocstatus ?? "init";
        });

        this.agentRuntimeLabel = createMemo(() => {
            nowMinute(); // re-evaluate every 60 s even without a status event
            const fullStatus = this.shellProcFullStatus();
            if (!fullStatus?.is_agent_pane) return null;
            if (fullStatus.shellprocstatus !== "running") return null;
            if (!fullStatus.spawn_ts_ms) return null;
            const elapsedMs = Date.now() - fullStatus.spawn_ts_ms;
            const elapsedHours = elapsedMs / 3_600_000;
            if (elapsedHours < 1) return null;
            const h = Math.floor(elapsedHours);
            const m = Math.floor((elapsedMs % 3_600_000) / 60_000);
            return `${h}h ${m}m`;
        });

        this.viewText = createMemo(() => {
            const rtn: HeaderElem[] = [];
            const isCmd = this.isCmdController();
            if (isCmd) {
                const blockMeta = this.blockAtom()?.meta;
                let cmdText = blockMeta?.["cmd"];
                let cmdArgs = blockMeta?.["cmd:args"];
                if (cmdArgs != null && Array.isArray(cmdArgs) && cmdArgs.length > 0) {
                    cmdText += " " + cmdArgs.join(" ");
                }
                rtn.push({
                    elemtype: "text",
                    text: cmdText,
                    noGrow: true,
                });
                const isRestarting = this.isRestarting();
                if (isRestarting) {
                    rtn.push({
                        elemtype: "iconbutton",
                        icon: "refresh",
                        iconColor: "var(--success-color)",
                        iconSpin: true,
                        title: "Restarting Command",
                        noAction: true,
                    });
                } else {
                    const fullShellProcStatus = this.shellProcFullStatus();
                    if (fullShellProcStatus?.shellprocstatus == "done") {
                        if (fullShellProcStatus?.shellprocexitcode == 0) {
                            rtn.push({
                                elemtype: "iconbutton",
                                icon: "check",
                                iconColor: "var(--success-color)",
                                title: "Command Exited Successfully",
                                noAction: true,
                            });
                        } else {
                            rtn.push({
                                elemtype: "iconbutton",
                                icon: "xmark-large",
                                iconColor: "var(--error-color)",
                                title: "Exit Code: " + fullShellProcStatus?.shellprocexitcode,
                                noAction: true,
                            });
                        }
                    }
                }
            }
            const isMI = atoms.isTermMultiInput();
            if (isMI && this.isBasicTerm()) {
                rtn.push({
                    elemtype: "textbutton",
                    text: "Multi Input ON",
                    className: "yellow",
                    title: "Input will be sent to all connected terminals (click to disable)",
                    onClick: () => {
                        setIsTermMultiInput(false);
                    },
                });
            }
            if (!isCmd) {
                const blockMeta = this.blockAtom()?.meta;
                const activity = blockMeta?.["term:osc_title"] as string | undefined;
                if (activity && activity.length > 0) {
                    rtn.push({
                        elemtype: "text",
                        text: activity,
                        className: "term-activity",
                    });
                }
            }
            return rtn;
        });

        this.manageConnection = createMemo(() => {
            const isCmd = this.isCmdController();
            return !isCmd;
        });

        this.termThemeNameAtom = useBlockAtom(blockId, "termthemeatom", () =>
            createMemo<string>(() => {
                return getOverrideConfigAtom(this.blockId, "term:theme")() ?? DefaultTermTheme;
            })
        );

        this.termTransparencyAtom = useBlockAtom(blockId, "termtransparencyatom", () =>
            createMemo<number>(() => {
                let value = getOverrideConfigAtom(this.blockId, "term:transparency")() ?? 0.5;
                return boundNumber(value, 0, 1);
            })
        );

        this.blockBg = createMemo(() => {
            const fullConfig = atoms.fullConfigAtom();
            const themeName = this.termThemeNameAtom();
            const termTransparency = this.termTransparencyAtom();
            const [_, bgcolor] = computeTheme(fullConfig, themeName, termTransparency);
            if (bgcolor != null) return { bg: bgcolor };
            return null;
        });

        this.connStatus = createMemo(() => {
            const blockData = this.blockAtom();
            const connName = blockData?.meta?.connection;
            const connAtom = getConnStatusAtom(connName);
            return connAtom();
        });

        this.termZoomAtom = useBlockAtom(blockId, "termzoomatom", () =>
            createMemo<number>(() => {
                const blockData = this.blockAtom();
                const zoomFactor = blockData?.meta?.["term:zoom"];
                if (zoomFactor == null) return 1.0;
                if (typeof zoomFactor !== "number" || isNaN(zoomFactor)) return 1.0;
                return Math.max(0.5, Math.min(2.0, zoomFactor));
            })
        );

        this.fontSizeAtom = useBlockAtom(blockId, "fontsizeatom", () =>
            createMemo<number>(() => {
                const blockData = this.blockAtom();
                const fsSettingsAtom = getSettingsKeyAtom("term:fontsize");
                const settingsFontSize = fsSettingsAtom();
                const connName = blockData?.meta?.connection;
                const fullConfig = atoms.fullConfigAtom();
                const connFontSize = fullConfig?.connections?.[connName]?.["term:fontsize"];
                const baseFontSize = blockData?.meta?.["term:fontsize"] ?? connFontSize ?? settingsFontSize ?? 15;
                if (typeof baseFontSize !== "number" || isNaN(baseFontSize) || baseFontSize < 4 || baseFontSize > 64) {
                    return 15;
                }
                const zoomFactor = this.termZoomAtom();
                const effectiveFontSize = baseFontSize * zoomFactor;
                return Math.max(4, Math.min(64, Math.round(effectiveFontSize)));
            })
        );

        this.noPadding = createSignalAtom(true);

        this.endIconButtons = createMemo(() => {
            const blockData = this.blockAtom();
            const shellProcStatus = this.shellProcStatus();
            const connStatus = this.connStatus();
            const isCmd = this.isCmdController();
            if (blockData?.meta?.["controller"] != "cmd" && shellProcStatus != "done") return [];
            if (connStatus?.status != "connected") return [];
            let iconName: string = null;
            let title: string = null;
            const noun = isCmd ? "Command" : "Shell";
            if (shellProcStatus == "init") {
                iconName = "play";
                title = "Click to Start " + noun;
            } else if (shellProcStatus == "running") {
                iconName = "refresh";
                title = noun + " Running. Click to Restart";
            } else if (shellProcStatus == "done") {
                iconName = "refresh";
                title = noun + " Exited. Click to Restart";
            }
            if (iconName == null) return [];
            const buttonDecl: IconButtonDecl = {
                elemtype: "iconbutton",
                icon: iconName,
                click: this.forceRestartController.bind(this),
                title: title,
            };
            return [buttonDecl];
        });

        // Voice input handle — streams transcript characters into the
        // PTY via the same blockinput path that keystrokes use, so
        // xterm input modes (echo, line-buffered, etc.) are honored.
        // No interim preview: the terminal has no affordance for it.
        this.voiceHandle = () => ({
            appendFinal: (text: string) => {
                const inputdata64 = stringToBase64(text);
                const cmd: BlockInputWSCommand = {
                    wscommand: "blockinput",
                    blockid: this.blockId,
                    inputdata64,
                };
                sendWSCommand(cmd);
            },
            setInterim: () => { /* terminal has no preview affordance */ },
        });

        const initialShellProcStatus = services.BlockService.GetControllerStatus(blockId);
        initialShellProcStatus.then((rts) => {
            this.updateShellProcStatus(rts);
        });
        this.shellProcStatusUnsubFn = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: WOS.makeORef("block", blockId),
            handler: (event) => {
                let bcRTS: BlockControllerRuntimeStatus = event.data;
                this.updateShellProcStatus(bcRTS);
            },
        });
    }

    get viewComponent(): ViewComponent {
        return _terminalViewComponent;
    }

    isBasicTerm(): boolean {
        const blockData = this.blockAtom();
        return blockData?.meta?.controller !== "cmd";
    }

    multiInputHandler(data: string) {
        let tvms = getAllBasicTermModels();
        tvms = tvms.filter((tvm) => tvm != this);
        if (tvms.length == 0) return;
        for (const tvm of tvms) {
            tvm.sendDataToController(data);
        }
    }

    // Max bytes per blockinput WebSocket frame. Keeps individual frames small
    // enough to avoid PTY write-buffer saturation on Windows ConPTY and large
    // single-write failures. Pastes above this threshold are split and sent
    // with a small inter-chunk delay to avoid flooding the backend channel.
    static readonly PASTE_CHUNK_BYTES = 4096;
    static readonly PASTE_CHUNK_DELAY_MS = 5;

    // Serializes ALL input on this block while a chunked paste is in flight:
    // every blockinput frame is sent with `seq: None`, so PTY order == send
    // order. A chunked paste sleeps 5 ms between chunks, so a keystroke or a
    // second paste arriving mid-paste must be queued behind it — otherwise it
    // would slip between chunks on the wire and corrupt the pasted content.
    private pasteChain: Promise<void> = Promise.resolve();
    // >0 while chunked-paste work is queued/running; gates the fast path below.
    private pasteInFlight = 0;
    // Monotonic id so concurrent/back-to-back pastes get distinct progress toasts.
    private pasteSeq = 0;

    private sendSingleFrame(data: string) {
        const inputdata64 = stringToBase64(data);
        const cmd: BlockInputWSCommand = { wscommand: "blockinput", blockid: this.blockId, inputdata64 };
        sendWSCommand(cmd);
    }

    sendDataToController(data: string) {
        // Use the blockinput wscommand, not the controllerinput RPC: blockinput
        // is processed inline & synchronously in the WS receive loop, so
        // consecutive keystrokes reach the PTY in TCP order. The RPC path is
        // dispatched concurrently by the engine and can reorder fast typing.
        const encoded = new TextEncoder().encode(data);
        const isLarge = encoded.length > TermViewModel.PASTE_CHUNK_BYTES;
        if (!isLarge && this.pasteInFlight === 0) {
            // Fast path: small input and no chunked paste in flight → send now.
            this.sendSingleFrame(data);
        } else {
            // Either a large paste, OR small input that must NOT overtake an
            // in-flight chunked paste. Queue on pasteChain to preserve wire order.
            // `.catch` keeps the chain alive if one item fails (a rejected promise
            // would otherwise skip every subsequent `.then`).
            this.pasteInFlight++;
            this.pasteChain = this.pasteChain
                .then(() => (isLarge ? this.sendDataChunked(encoded) : this.sendSingleFrame(data)))
                .catch(() => {})
                .finally(() => {
                    this.pasteInFlight--;
                });
        }
    }

    private async sendDataChunked(encoded: Uint8Array) {
        const chunkSize = TermViewModel.PASTE_CHUNK_BYTES;
        const decoder = new TextDecoder();
        const kb = Math.round(encoded.length / 1024);
        // Unique per paste so back-to-back/overlapping pastes don't share (and
        // prematurely dismiss) one another's progress toast.
        const toastId = `paste-progress-${this.blockId}-${++this.pasteSeq}`;
        const showToast = kb >= 8;
        if (showToast) {
            pushNotification({
                id: toastId,
                icon: "clipboard",
                title: "Pasting…",
                message: `Sending ${kb} KB in chunks`,
                timestamp: new Date().toISOString(),
                type: "info",
            });
        }
        // try/finally so the toast is always removed even if sendWSCommand throws
        // mid-paste — otherwise the "Pasting…" notification would leak forever.
        try {
            for (let offset = 0; offset < encoded.length; offset += chunkSize) {
                const slice = encoded.subarray(offset, offset + chunkSize);
                const isLast = offset + chunkSize >= encoded.length;
                // Stream-decode so a multi-byte UTF-8 character straddling a chunk
                // boundary is not split into two U+FFFD replacements: with
                // `{ stream: true }` the decoder defers the trailing partial bytes
                // to the next chunk. The concatenation of all chunks reconstructs
                // the original bytes exactly; the final chunk flushes (stream:false).
                const text = decoder.decode(slice, { stream: !isLast });
                if (text.length > 0) {
                    const inputdata64 = stringToBase64(text);
                    const cmd: BlockInputWSCommand = { wscommand: "blockinput", blockid: this.blockId, inputdata64 };
                    sendWSCommand(cmd);
                }
                if (!isLast) {
                    await sleep(TermViewModel.PASTE_CHUNK_DELAY_MS);
                }
            }
        } finally {
            if (showToast) {
                removeNotificationById(toastId);
            }
        }
    }

    triggerRestartAtom() {
        this.isRestarting._set(true);
        setTimeout(() => {
            this.isRestarting._set(false);
        }, 300);
    }

    updateShellProcStatus(fullStatus: BlockControllerRuntimeStatus) {
        if (fullStatus == null) return;
        const curStatus = this.shellProcFullStatus();
        if (curStatus == null || curStatus.version < fullStatus.version) {
            this.shellProcFullStatus._set(fullStatus);
        }
    }

    dispose() {
        DefaultRouter.unregisterRoute(makeFeBlockRouteId(this.blockId));
        if (this.shellProcStatusUnsubFn) {
            this.shellProcStatusUnsubFn();
        }
    }

    giveFocus(): boolean {
        if (this.searchAtoms && this.searchAtoms.isOpen()) {
            console.log("search is open, not giving focus");
            return true;
        }
        let termMode = this.termMode();
        if (termMode == "term") {
            if (this.termRef?.current?.terminal) {
                this.termRef.current.terminal.focus();
                return true;
            }
        }
        return false;
    }

    keyDownHandler(waveEvent: WaveKeyboardEvent): boolean {
        return false;
    }

    handleTerminalKeydown(event: KeyboardEvent): boolean {
        const waveEvent = keyutil.adaptFromReactOrNativeKeyEvent(event);
        if (waveEvent.type != "keydown") return true;
        if (this.keyDownHandler(waveEvent)) {
            event.preventDefault();
            event.stopPropagation();
            return false;
        }
        if (keyutil.checkKeyPressed(waveEvent, "Shift:Enter")) {
            const shiftEnterNewlineAtom = getOverrideConfigAtom(this.blockId, "term:shiftenternewline");
            const shiftEnterNewlineEnabled = shiftEnterNewlineAtom() ?? false;
            if (shiftEnterNewlineEnabled) {
                this.sendDataToController("\u001b\n");
                event.preventDefault();
                event.stopPropagation();
                return false;
            }
        }
        if (keyutil.checkKeyPressed(waveEvent, "Ctrl:Shift:v")) {
            clipboardReadText()
                .then((text) => {
                    this.termRef.current?.terminal.paste(text);
                })
                .catch((e) => console.log("clipboard read failed", e));
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(waveEvent, "Ctrl:Shift:c")) {
            const sel = this.termRef.current?.terminal.getSelection();
            clipboardWriteText(sel).catch((e) => console.log("clipboard write failed", e));
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(waveEvent, "Cmd:k")) {
            event.preventDefault();
            event.stopPropagation();
            this.termRef.current?.terminal?.clear();
            return false;
        }
        const shellProcStatus = this.shellProcStatus();
        if (shellProcStatus == "done" && keyutil.checkKeyPressed(waveEvent, "Enter")) {
            this.forceRestartController();
            return false;
        }
        const appHandled = appHandleKeyDown(waveEvent);
        if (appHandled) {
            event.preventDefault();
            event.stopPropagation();
            return false;
        }
        return true;
    }

    setTerminalTheme(themeName: string) {
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", this.blockId),
            meta: { "term:theme": themeName },
        });
    }

    forceRestartController() {
        if (this.isRestarting()) return;
        this.triggerRestartAtom();
        const termsize = {
            rows: this.termRef.current?.terminal?.rows,
            cols: this.termRef.current?.terminal?.cols,
        };
        const prtn = RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: atoms.staticTabId(),
            blockid: this.blockId,
            forcerestart: true,
            rtopts: { termsize: termsize },
        });
        prtn.catch((e) => console.log("error controller resync (force restart)", e));
    }

    getSettingsMenuItems(): ContextMenuItem[] {
        return buildSettingsMenuItems(this);
    }
}

function getAllBasicTermModels(): TermViewModel[] {
    const allBCMs = getAllBlockComponentModels();
    const rtn: TermViewModel[] = [];
    for (const bcm of allBCMs) {
        if (bcm.viewModel?.viewType != "term") continue;
        const termVM = bcm.viewModel as TermViewModel;
        if (termVM.isBasicTerm()) {
            rtn.push(termVM);
        }
    }
    return rtn;
}

function makeTerminalModel(blockId: string, nodeModel: BlockNodeModel): TermViewModel {
    return new TermViewModel(blockId, nodeModel);
}

export { makeTerminalModel, TermViewModel };
