// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { appHandleKeyDown } from "@/app/store/keymodel";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { sendWSCommand } from "@/app/store/ws";
import { makeFeBlockRouteId } from "@/app/store/rpc-router";
import { DefaultRouter, TabRpcClient } from "@/app/store/rpc-util";
import { TermRpcClient } from "@/app/view/term/term-rpc";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";
import {
    atoms,
    getConnStatusAtom,
    getOverrideConfigAtom,
    getSettingsKeyAtom,
    setIsTermMultiInput,
    MOS,
} from "@/store/global";
import * as services from "@/store/services";
import * as keyutil from "@/util/keyutil";
import { boundNumber, createSignalAtom, stringToBase64 } from "@/util/util";
import type { SignalAtom } from "@/util/util";
import { createMemo, createSignal, type Accessor } from "solid-js";

// Ticks every 60 s so agentRuntimeLabel memos re-evaluate without waiting for a status event.
// globalThis survives HMR module re-evaluation — prevents duplicate interval leak.
const [nowMinute, setNowMinute] = createSignal(Math.floor(Date.now() / 60_000));
if ((globalThis as any).__nowMinuteInterval != null) clearInterval((globalThis as any).__nowMinuteInterval);
(globalThis as any).__nowMinuteInterval = setInterval(() => setNowMinute(Math.floor(Date.now() / 60_000)), 60_000);
import { resolveTermScrollSensitivity } from "./termscrollsensitivity";
import { computeTheme, DefaultTermTheme, termViewName } from "./termutil";
import { BlockInputSender } from "./block-input-sender";
import { TermWrap } from "./termwrap";
import { basicTermModels, termModels } from "./term-models";
import { buildSettingsMenuItems } from "./termSettingsMenu";

/** The terminal's state behind its native pane tab (`terminalPaneTab`,
 *  term.tsx). */
class TermViewModel {
    viewType: string;
    connected: boolean;
    termRef: { current: TermWrap | null } = { current: null };
    /** The block's meta — the host context's, reactive. */
    meta: Accessor<MetaType | undefined>;
    /** Whether this tab's pane has focus (multi-input follows it). */
    isFocused: Accessor<boolean>;
    termMode: () => string;
    blockId: string;
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
    scrollSensitivityAtom: () => number;
    endIconButtons: () => IconButtonDecl[];
    shellProcFullStatus: SignalAtom<BlockControllerRuntimeStatus>;
    shellProcStatus: () => string;
    shellProcStatusUnsubFn: () => void;
    isCmdController: () => boolean;
    isRestarting: SignalAtom<boolean>;
    agentRuntimeLabel: () => string | null;
    searchAtoms?: SearchAtoms;
    voiceHandle: () => PaneVoiceHandle;
    private unregisterModel: () => void;
    private ctx: PaneTabHostContext;

    // A native pane tab (Pane Tab contract Phase 2c): built by `create(ctx)`
    // (term.tsx) in its own reactive root, so its memos are plain memos —
    // they used to be parked in the per-block atom cache to
    // outlive effect re-runs host rule 8 has since fixed. Its own block's
    // meta comes from, and goes to, the host context.
    constructor(ctx: PaneTabHostContext) {
        const blockId = ctx.blockId;
        this.ctx = ctx;
        this.viewType = "term";
        this.blockId = blockId;
        this.meta = ctx.meta;
        this.isFocused = ctx.isFocused;
        this.termRpcClient = new TermRpcClient(blockId, this);
        DefaultRouter.registerRoute(makeFeBlockRouteId(blockId), this.termRpcClient);

        this.termMode = createMemo(() => this.meta()?.["term:mode"] ?? "term");

        this.isRestarting = createSignalAtom(false);

        this.viewName = createMemo(() => termViewName(this.meta()));

        this.isCmdController = createMemo(() => this.meta()?.controller == "cmd");

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
                const blockMeta = this.meta();
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
                const blockMeta = this.meta();
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

        this.termThemeNameAtom = createMemo<string>(
            () => getOverrideConfigAtom(this.blockId, "term:theme")() ?? DefaultTermTheme
        );

        this.termTransparencyAtom = createMemo<number>(() =>
            boundNumber(getOverrideConfigAtom(this.blockId, "term:transparency")() ?? 0.5, 0, 1)
        );

        // term:scrollsensitivity has no per-block override (unlike
        // transparency/fontSize above) — it's intentionally global-only
        // (SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31.md §4.1), so
        // this reads the plain setting, not getOverrideConfigAtom. Resolved
        // through the same shared function termwrap.ts's constructor and
        // AgentShellSubblock.tsx's own atom use, so a configured value means
        // the same thing everywhere (REPORT_TERMINAL_SCROLL_SENSITIVITY_NOT_LIVE_2026_09_22.md).
        this.scrollSensitivityAtom = createMemo<number>(() =>
            resolveTermScrollSensitivity(getSettingsKeyAtom("term:scrollsensitivity")())
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
            const connName = this.meta()?.connection;
            const connAtom = getConnStatusAtom(connName);
            return connAtom();
        });

        this.termZoomAtom = createMemo<number>(() => {
            const zoomFactor = this.meta()?.["term:zoom"];
            if (zoomFactor == null) return 1.0;
            if (typeof zoomFactor !== "number" || isNaN(zoomFactor)) return 1.0;
            return Math.max(0.5, Math.min(2.0, zoomFactor));
        });

        this.fontSizeAtom = createMemo<number>(() => {
            const meta = this.meta();
            const settingsFontSize = getSettingsKeyAtom("term:fontsize")();
            const connName = meta?.connection;
            const fullConfig = atoms.fullConfigAtom();
            const connFontSize = fullConfig?.connections?.[connName]?.["term:fontsize"];
            const baseFontSize = meta?.["term:fontsize"] ?? connFontSize ?? settingsFontSize ?? 15;
            if (typeof baseFontSize !== "number" || isNaN(baseFontSize) || baseFontSize < 4 || baseFontSize > 64) {
                return 15;
            }
            const effectiveFontSize = baseFontSize * this.termZoomAtom();
            return Math.max(4, Math.min(64, Math.round(effectiveFontSize)));
        });

        this.endIconButtons = createMemo(() => {
            const meta = this.meta();
            const shellProcStatus = this.shellProcStatus();
            const connStatus = this.connStatus();
            const isCmd = this.isCmdController();
            if (meta?.["controller"] != "cmd" && shellProcStatus != "done") return [];
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
        this.shellProcStatusUnsubFn = muxEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: MOS.makeORef("block", blockId),
            handler: (event) => {
                let bcRTS: BlockControllerRuntimeStatus = event.data;
                this.updateShellProcStatus(bcRTS);
            },
        });

        // Last, once every field exists: registering notifies readers of
        // this block (the pane chrome's runtime badge, multi-input), which
        // then read those fields (ReAgent P1 on #3807).
        this.unregisterModel = termModels.register(blockId, this);
    }

    isBasicTerm(): boolean {
        return this.meta()?.controller !== "cmd";
    }

    /** Writes this terminal's own block meta. */
    setMeta(patch: MetaType): Promise<void> {
        return this.ctx.setMeta(patch);
    }

    /** xterm's own selection, which survives the right-click that clears
     *  the page's (the pane menu's Copy). */
    getSelection(): string {
        return this.termRef.current?.terminal?.getSelection() ?? "";
    }

    paste(text: string) {
        this.termRef.current?.terminal?.paste(text);
    }

    multiInputHandler(data: string) {
        const tvms = basicTermModels().filter((tvm) => tvm != this);
        if (tvms.length == 0) return;
        for (const tvm of tvms) {
            tvm.sendDataToController(data);
        }
    }

    // Ordered, paced blockinput sender (chunks large pastes, serializes input
    // behind an in-flight chunked paste). Shared with the agent pane's shell
    // drawer — see block-input-sender.ts. Created lazily: blockId is assigned
    // in the constructor, after field initializers have run.
    private _inputSender: BlockInputSender | null = null;

    sendDataToController(data: string) {
        (this._inputSender ??= new BlockInputSender(this.blockId)).send(data);
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
        this.unregisterModel();
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

    keyDownHandler(muxEvent: MuxKeyboardEvent): boolean {
        return false;
    }

    handleTerminalKeydown(event: KeyboardEvent): boolean {
        const muxEvent = keyutil.adaptFromReactOrNativeKeyEvent(event);
        if (muxEvent.type != "keydown") return true;
        if (this.keyDownHandler(muxEvent)) {
            event.preventDefault();
            event.stopPropagation();
            return false;
        }
        if (keyutil.checkKeyPressed(muxEvent, "Shift:Enter")) {
            const shiftEnterNewlineAtom = getOverrideConfigAtom(this.blockId, "term:shiftenternewline");
            const shiftEnterNewlineEnabled = shiftEnterNewlineAtom() ?? false;
            if (shiftEnterNewlineEnabled) {
                this.sendDataToController("\u001b\n");
                event.preventDefault();
                event.stopPropagation();
                return false;
            }
        }
        if (keyutil.checkKeyPressed(muxEvent, "Ctrl:Shift:v")) {
            clipboardReadText()
                .then((text) => {
                    this.termRef.current?.terminal.paste(text);
                })
                .catch((e) => console.log("clipboard read failed", e));
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(muxEvent, "Ctrl:Shift:c")) {
            const sel = this.termRef.current?.terminal.getSelection();
            clipboardWriteText(sel).catch((e) => console.log("clipboard write failed", e));
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(muxEvent, "Cmd:k")) {
            event.preventDefault();
            event.stopPropagation();
            this.termRef.current?.terminal?.clear();
            return false;
        }
        const shellProcStatus = this.shellProcStatus();
        if (shellProcStatus == "done" && keyutil.checkKeyPressed(muxEvent, "Enter")) {
            this.forceRestartController();
            return false;
        }
        const appHandled = appHandleKeyDown(muxEvent);
        if (appHandled) {
            event.preventDefault();
            event.stopPropagation();
            return false;
        }
        return true;
    }

    setTerminalTheme(themeName: string) {
        void this.setMeta({ "term:theme": themeName });
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

export { TermViewModel };
