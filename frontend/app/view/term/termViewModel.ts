// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { BlockNodeModel } from "@/app/block/blocktypes";
import { appHandleKeyDown } from "@/app/store/keymodel";
import { waveEventSubscribe } from "@/app/store/wps";
import { RpcApi } from "@/app/store/wshclientapi";
import { makeFeBlockRouteId } from "@/app/store/wshrouter";
import { DefaultRouter, TabRpcClient } from "@/app/store/wshrpcutil";
import { TermWshClient } from "@/app/view/term/term-wsh";
import { VDomModel } from "@/app/view/vdom/vdom-model";
import {
    atoms,
    getAllBlockComponentModels,
    getBlockComponentModel,
    getBlockMetaKeyAtom,
    getConnStatusAtom,
    getOverrideConfigAtom,
    getSettingsKeyAtom,
    globalStore,
    useBlockAtom,
    WOS,
} from "@/store/global";
import * as services from "@/store/services";
import * as keyutil from "@/util/keyutil";
import { boundNumber, stringToBase64 } from "@/util/util";
import { computeTheme, DefaultTermTheme } from "./termutil";
import { TermWrap } from "./termwrap";
import { buildSettingsMenuItems } from "./termSettingsMenu";
import * as jotai from "jotai";

let _terminalViewComponent: ViewComponent = null;

export function setTerminalViewComponent(component: ViewComponent) {
    _terminalViewComponent = component;
}

class TermViewModel implements ViewModel {
    viewType: string;
    nodeModel: BlockNodeModel;
    connected: boolean;
    termRef: React.MutableRefObject<TermWrap> = { current: null };
    blockAtom: jotai.Atom<Block>;
    termMode: jotai.Atom<string>;
    blockId: string;
    viewIcon: jotai.Atom<string>;
    viewName: jotai.Atom<string>;
    viewText: jotai.Atom<HeaderElem[]>;
    blockBg: jotai.Atom<MetaType>;
    manageConnection: jotai.Atom<boolean>;
    filterOutNowsh?: jotai.Atom<boolean>;
    connStatus: jotai.Atom<ConnStatus>;
    termWshClient: TermWshClient;
    vdomBlockId: jotai.Atom<string>;
    vdomToolbarBlockId: jotai.Atom<string>;
    vdomToolbarTarget: jotai.PrimitiveAtom<VDomTargetToolbar>;
    fontSizeAtom: jotai.Atom<number>;
    termZoomAtom: jotai.Atom<number>;
    termThemeNameAtom: jotai.Atom<string>;
    termTransparencyAtom: jotai.Atom<number>;
    noPadding: jotai.PrimitiveAtom<boolean>;
    endIconButtons: jotai.Atom<IconButtonDecl[]>;
    shellProcFullStatus: jotai.PrimitiveAtom<BlockControllerRuntimeStatus>;
    shellProcStatus: jotai.Atom<string>;
    shellProcStatusUnsubFn: () => void;
    isCmdController: jotai.Atom<boolean>;
    isRestarting: jotai.PrimitiveAtom<boolean>;
    searchAtoms?: SearchAtoms;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.viewType = "term";
        this.blockId = blockId;
        this.termWshClient = new TermWshClient(blockId, this);
        DefaultRouter.registerRoute(makeFeBlockRouteId(blockId), this.termWshClient);
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.vdomBlockId = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            return blockData?.meta?.["term:vdomblockid"];
        });
        this.vdomToolbarBlockId = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            return blockData?.meta?.["term:vdomtoolbarblockid"];
        });
        this.vdomToolbarTarget = jotai.atom<VDomTargetToolbar>(null) as jotai.PrimitiveAtom<VDomTargetToolbar>;
        this.termMode = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            return blockData?.meta?.["term:mode"] ?? "term";
        });
        this.isRestarting = jotai.atom(false);
        this.viewIcon = jotai.atom((get) => {
            const termMode = get(this.termMode);
            if (termMode == "vdom") {
                return "bolt";
            }
            const isCmd = get(this.isCmdController);
            if (isCmd) {
            }
            return "terminal";
        });
        this.viewName = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const termMode = get(this.termMode);
            if (termMode == "vdom") {
                return "Wave App";
            }
            if (blockData?.meta?.controller == "cmd") {
                return "";
            }
            return "Terminal";
        });
        this.viewText = jotai.atom((get) => {
            const termMode = get(this.termMode);
            if (termMode == "vdom") {
                return [
                    {
                        elemtype: "iconbutton",
                        icon: "square-terminal",
                        title: "Switch back to Terminal",
                        click: () => {
                            this.setTermMode("term");
                        },
                    },
                ];
            }
            const vdomBlockId = get(this.vdomBlockId);
            const rtn: HeaderElem[] = [];
            if (vdomBlockId) {
                rtn.push({
                    elemtype: "iconbutton",
                    icon: "bolt",
                    title: "Switch to Wave App",
                    click: () => {
                        this.setTermMode("vdom");
                    },
                });
            }
            const isCmd = get(this.isCmdController);
            if (isCmd) {
                const blockMeta = get(this.blockAtom)?.meta;
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
                const isRestarting = get(this.isRestarting);
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
                    const fullShellProcStatus = get(this.shellProcFullStatus);
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
            const isMI = get(atoms.isTermMultiInput);
            if (isMI && this.isBasicTerm(get)) {
                rtn.push({
                    elemtype: "textbutton",
                    text: "Multi Input ON",
                    className: "yellow",
                    title: "Input will be sent to all connected terminals (click to disable)",
                    onClick: () => {
                        globalStore.set(atoms.isTermMultiInput, false);
                    },
                });
            }
            // Display term:activity (Claude Code's current activity summary)
            if (!isCmd) {
                const blockMeta = get(this.blockAtom)?.meta;
                const activity = blockMeta?.["term:activity"] as string | undefined;
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
        this.manageConnection = jotai.atom((get) => {
            const termMode = get(this.termMode);
            if (termMode == "vdom") {
                return false;
            }
            const isCmd = get(this.isCmdController);
            if (isCmd) {
                return false;
            }
            return true;
        });
        this.filterOutNowsh = jotai.atom(false);
        this.termThemeNameAtom = useBlockAtom(blockId, "termthemeatom", () => {
            return jotai.atom<string>((get) => {
                return get(getOverrideConfigAtom(this.blockId, "term:theme")) ?? DefaultTermTheme;
            });
        });
        this.termTransparencyAtom = useBlockAtom(blockId, "termtransparencyatom", () => {
            return jotai.atom<number>((get) => {
                let value = get(getOverrideConfigAtom(this.blockId, "term:transparency")) ?? 0.5;
                return boundNumber(value, 0, 1);
            });
        });
        this.blockBg = jotai.atom((get) => {
            const fullConfig = get(atoms.fullConfigAtom);
            const themeName = get(this.termThemeNameAtom);
            const termTransparency = get(this.termTransparencyAtom);
            const [_, bgcolor] = computeTheme(fullConfig, themeName, termTransparency);
            if (bgcolor != null) {
                return { bg: bgcolor };
            }
            return null;
        });
        this.connStatus = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const connName = blockData?.meta?.connection;
            const connAtom = getConnStatusAtom(connName);
            return get(connAtom);
        });
        this.termZoomAtom = useBlockAtom(blockId, "termzoomatom", () => {
            return jotai.atom<number>((get) => {
                const blockData = get(this.blockAtom);
                const zoomFactor = blockData?.meta?.["term:zoom"];

                // Validate range
                if (zoomFactor == null) {
                    return 1.0; // Default 100%
                }
                if (typeof zoomFactor !== "number" || isNaN(zoomFactor)) {
                    return 1.0;
                }
                // Clamp to safe range (0.5-2.0)
                return Math.max(0.5, Math.min(2.0, zoomFactor));
            });
        });
        this.fontSizeAtom = useBlockAtom(blockId, "fontsizeatom", () => {
            return jotai.atom<number>((get) => {
                const blockData = get(this.blockAtom);
                const fsSettingsAtom = getSettingsKeyAtom("term:fontsize");
                const settingsFontSize = get(fsSettingsAtom);
                const connName = blockData?.meta?.connection;
                const fullConfig = get(atoms.fullConfigAtom);
                const connFontSize = fullConfig?.connections?.[connName]?.["term:fontsize"];

                // Get base font size (existing logic)
                const baseFontSize = blockData?.meta?.["term:fontsize"] ?? connFontSize ?? settingsFontSize ?? 12;

                // Validate base font size
                if (typeof baseFontSize !== "number" || isNaN(baseFontSize) || baseFontSize < 4 || baseFontSize > 64) {
                    return 12;
                }

                // Apply zoom factor
                const zoomFactor = get(this.termZoomAtom);
                const effectiveFontSize = baseFontSize * zoomFactor;

                // Final validation (clamp to 4-64px range)
                return Math.max(4, Math.min(64, Math.round(effectiveFontSize)));
            });
        });
        this.noPadding = jotai.atom(true);
        this.endIconButtons = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const shellProcStatus = get(this.shellProcStatus);
            const connStatus = get(this.connStatus);
            const isCmd = get(this.isCmdController);
            if (blockData?.meta?.["controller"] != "cmd" && shellProcStatus != "done") {
                return [];
            }
            if (connStatus?.status != "connected") {
                return [];
            }
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
            if (iconName == null) {
                return [];
            }
            const buttonDecl: IconButtonDecl = {
                elemtype: "iconbutton",
                icon: iconName,
                click: this.forceRestartController.bind(this),
                title: title,
            };
            const rtn = [buttonDecl];
            return rtn;
        });
        this.isCmdController = jotai.atom((get) => {
            const controllerMetaAtom = getBlockMetaKeyAtom(this.blockId, "controller");
            return get(controllerMetaAtom) == "cmd";
        });
        this.shellProcFullStatus = jotai.atom(null) as jotai.PrimitiveAtom<BlockControllerRuntimeStatus>;
        const initialShellProcStatus = services.BlockService.GetControllerStatus(blockId);
        initialShellProcStatus.then((rts) => {
            this.updateShellProcStatus(rts);
        });
        this.shellProcStatusUnsubFn = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", blockId),
            handler: (event) => {
                let bcRTS: BlockControllerRuntimeStatus = event.data;
                this.updateShellProcStatus(bcRTS);
            },
        });
        this.shellProcStatus = jotai.atom((get) => {
            const fullStatus = get(this.shellProcFullStatus);
            return fullStatus?.shellprocstatus ?? "init";
        });
    }

    get viewComponent(): ViewComponent {
        return _terminalViewComponent;
    }

    isBasicTerm(getFn: jotai.Getter): boolean {
        // needs to match "const isBasicTerm" in TerminalView()
        const termMode = getFn(this.termMode);
        if (termMode == "vdom") {
            return false;
        }
        const blockData = getFn(this.blockAtom);
        if (blockData?.meta?.controller == "cmd") {
            return false;
        }
        return true;
    }

    multiInputHandler(data: string) {
        let tvms = getAllBasicTermModels();
        // filter out "this" from the list
        tvms = tvms.filter((tvm) => tvm != this);
        if (tvms.length == 0) {
            return;
        }
        for (const tvm of tvms) {
            tvm.sendDataToController(data);
        }
    }

    sendDataToController(data: string) {
        const b64data = stringToBase64(data);
        RpcApi.ControllerInputCommand(TabRpcClient, { blockid: this.blockId, inputdata64: b64data });
    }

    setTermMode(mode: "term" | "vdom") {
        if (mode == "term") {
            mode = null;
        }
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", this.blockId),
            meta: { "term:mode": mode },
        });
    }

    triggerRestartAtom() {
        globalStore.set(this.isRestarting, true);
        setTimeout(() => {
            globalStore.set(this.isRestarting, false);
        }, 300);
    }

    updateShellProcStatus(fullStatus: BlockControllerRuntimeStatus) {
        if (fullStatus == null) {
            return;
        }
        const curStatus = globalStore.get(this.shellProcFullStatus);
        if (curStatus == null || curStatus.version < fullStatus.version) {
            globalStore.set(this.shellProcFullStatus, fullStatus);
        }
    }

    getVDomModel(): VDomModel {
        const vdomBlockId = globalStore.get(this.vdomBlockId);
        if (!vdomBlockId) {
            return null;
        }
        const bcm = getBlockComponentModel(vdomBlockId);
        if (!bcm) {
            return null;
        }
        return bcm.viewModel as VDomModel;
    }

    getVDomToolbarModel(): VDomModel {
        const vdomToolbarBlockId = globalStore.get(this.vdomToolbarBlockId);
        if (!vdomToolbarBlockId) {
            return null;
        }
        const bcm = getBlockComponentModel(vdomToolbarBlockId);
        if (!bcm) {
            return null;
        }
        return bcm.viewModel as VDomModel;
    }

    dispose() {
        DefaultRouter.unregisterRoute(makeFeBlockRouteId(this.blockId));
        if (this.shellProcStatusUnsubFn) {
            this.shellProcStatusUnsubFn();
        }
    }

    giveFocus(): boolean {
        if (this.searchAtoms && globalStore.get(this.searchAtoms.isOpen)) {
            console.log("search is open, not giving focus");
            return true;
        }
        let termMode = globalStore.get(this.termMode);
        if (termMode == "term") {
            if (this.termRef?.current?.terminal) {
                this.termRef.current.terminal.focus();
                return true;
            }
        }
        return false;
    }

    keyDownHandler(waveEvent: WaveKeyboardEvent): boolean {
        if (keyutil.checkKeyPressed(waveEvent, "Cmd:Escape")) {
            const blockAtom = WOS.getWaveObjectAtom<Block>(`block:${this.blockId}`);
            const blockData = globalStore.get(blockAtom);
            const newTermMode = blockData?.meta?.["term:mode"] == "vdom" ? null : "vdom";
            const vdomBlockId = globalStore.get(this.vdomBlockId);
            if (newTermMode == "vdom" && !vdomBlockId) {
                return;
            }
            this.setTermMode(newTermMode);
            return true;
        }
        const blockData = globalStore.get(this.blockAtom);
        if (blockData.meta?.["term:mode"] == "vdom") {
            const vdomModel = this.getVDomModel();
            return vdomModel?.keyDownHandler(waveEvent);
        }
        return false;
    }

    handleTerminalKeydown(event: KeyboardEvent): boolean {
        const waveEvent = keyutil.adaptFromReactOrNativeKeyEvent(event);
        if (waveEvent.type != "keydown") {
            return true;
        }
        if (this.keyDownHandler(waveEvent)) {
            event.preventDefault();
            event.stopPropagation();
            return false;
        }
        // deal with terminal specific keybindings
        if (keyutil.checkKeyPressed(waveEvent, "Shift:Enter")) {
            // Check if shift+enter newline is enabled via config
            const shiftEnterNewlineAtom = getOverrideConfigAtom(this.blockId, "term:shiftenternewline");
            const shiftEnterNewlineEnabled = globalStore.get(shiftEnterNewlineAtom) ?? false;
            if (shiftEnterNewlineEnabled) {
                // Support for claude code - send escape sequence + newline instead of carriage return
                this.sendDataToController("\u001b\n");
                event.preventDefault();
                event.stopPropagation();
                return false;
            }
        }
        if (keyutil.checkKeyPressed(waveEvent, "Ctrl:Shift:v")) {
            const p = navigator.clipboard.readText();
            p.then((text) => {
                this.termRef.current?.terminal.paste(text);
            });
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(waveEvent, "Ctrl:Shift:c")) {
            const sel = this.termRef.current?.terminal.getSelection();
            navigator.clipboard.writeText(sel);
            event.preventDefault();
            event.stopPropagation();
            return false;
        } else if (keyutil.checkKeyPressed(waveEvent, "Cmd:k")) {
            event.preventDefault();
            event.stopPropagation();
            this.termRef.current?.terminal?.clear();
            return false;
        }
        const shellProcStatus = globalStore.get(this.shellProcStatus);
        if ((shellProcStatus == "done" || shellProcStatus == "init") && keyutil.checkKeyPressed(waveEvent, "Enter")) {
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
        if (globalStore.get(this.isRestarting)) {
            return;
        }
        this.triggerRestartAtom();
        const termsize = {
            rows: this.termRef.current?.terminal?.rows,
            cols: this.termRef.current?.terminal?.cols,
        };
        const prtn = RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: globalStore.get(atoms.staticTabId),
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
        if (bcm.viewModel?.viewType != "term") {
            continue;
        }
        const termVM = bcm.viewModel as TermViewModel;
        if (termVM.isBasicTerm(globalStore.get)) {
            rtn.push(termVM);
        }
    }
    return rtn;
}

function makeTerminalModel(blockId: string, nodeModel: BlockNodeModel): TermViewModel {
    return new TermViewModel(blockId, nodeModel);
}

export { makeTerminalModel, TermViewModel };
