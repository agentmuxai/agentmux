// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { focusManager } from "@/app/store/focusManager";
import {
    atoms,
    createBlock,
    createBlockSplitHorizontally,
    createBlockSplitVertically,
    createTab,
    getAllBlockComponentModels,
    getApi,
    getBlockComponentModel,
    getFocusedBlockId,
    getSettingsKeyAtom,
    globalStore,
    refocusNode,
    replaceBlock,
    setActiveTab,
    setControlShiftDelayAtom,
    setIsTermMultiInput,
    WOS,
} from "@/app/store/global";
import { WorkspaceService } from "@/app/store/services";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { zoomIn, zoomOut, zoomReset } from "@/app/store/zoom.platform";
import { TabBarModel } from "@/app/tab/tabbar-model";
import { deleteLayoutModelForTab, getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import * as keyutil from "@/util/keyutil";
import { CHORD_TIMEOUT } from "@/util/sharedconst";
import { fireAndForget } from "@/util/util";
import { createSignal } from "solid-js";
import { modalsModel } from "./modalmodel";

// Debug logging function - writes to file
const DEBUG_LOG_PATH = "C:/Systems/agentmux-debug.log";

function stringToBase64(str: string): string {
    const bytes = new TextEncoder().encode(str);
    let binary = "";
    for (let i = 0; i < bytes.length; i++) {
        binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
}

function debugLog(message: string, data?: unknown): void {
    const timestamp = new Date().toISOString();
    const logLine = `[${timestamp}] [KEYMODEL] ${message}${data !== undefined ? ": " + JSON.stringify(data) : ""}\n`;
    fireAndForget(async () => {
        try {
            await RpcApi.FileAppendCommand(TabRpcClient, {
                info: { path: DEBUG_LOG_PATH },
                data64: stringToBase64(logLine),
            });
        } catch (e) {
            console.error("Failed to write debug log:", e);
        }
    });
}

type KeyHandler = (event: WaveKeyboardEvent) => boolean;

const [simpleControlShift, setSimpleControlShift] = createSignal(false);
const globalKeyMap = new Map<string, (waveEvent: WaveKeyboardEvent) => boolean>();
const globalChordMap = new Map<string, Map<string, KeyHandler>>();
let globalKeybindingsDisabled = false;

// track current chord state and timeout (for resetting)
let activeChord: string | null = null;
let chordTimeout: NodeJS.Timeout = null;

function resetChord() {
    activeChord = null;
    if (chordTimeout) {
        clearTimeout(chordTimeout);
        chordTimeout = null;
    }
}

function setActiveChord(activeChordArg: string) {
    getApi().setKeyboardChordMode();
    if (chordTimeout) {
        clearTimeout(chordTimeout);
    }
    activeChord = activeChordArg;
    chordTimeout = setTimeout(() => resetChord(), CHORD_TIMEOUT);
}

export function keyboardMouseDownHandler(e: MouseEvent) {
    if (!e.ctrlKey || !e.shiftKey) {
        unsetControlShift();
    }
}

function getFocusedBlockInStaticTab() {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    return focusedNode?.data?.blockId;
}

function getSimpleControlShiftAtom() {
    return simpleControlShift;
}

function setControlShift() {
    setSimpleControlShift(true);
    setTimeout(() => {
        if (simpleControlShift()) {
            setControlShiftDelayAtom(true);
        }
    }, 400);
}

function unsetControlShift() {
    setSimpleControlShift(false);
    setControlShiftDelayAtom(false);
}

function disableGlobalKeybindings() {
    globalKeybindingsDisabled = true;
}

function enableGlobalKeybindings() {
    globalKeybindingsDisabled = false;
}

function shouldDispatchToBlock(e: WaveKeyboardEvent): boolean {
    if (atoms.modalOpen()) {
        return false;
    }
    const activeElem = document.activeElement;
    if (activeElem != null && activeElem instanceof HTMLElement) {
        if (activeElem.tagName == "INPUT" || activeElem.tagName == "TEXTAREA" || activeElem.contentEditable == "true") {
            if (activeElem.classList.contains("dummy-focus") || activeElem.classList.contains("dummy")) {
                return true;
            }
            if (keyutil.isInputEvent(e)) {
                return false;
            }
            return true;
        }
    }
    return true;
}

function getStaticTabBlockCount(): number {
    const tabId = atoms.activeTabId();
    const tabORef = WOS.makeORef("tab", tabId);
    const tabAtom = WOS.getWaveObjectAtom<Tab>(tabORef);
    const tabData = tabAtom();
    return tabData?.blockids?.length ?? 0;
}

function isStaticTabPinned(): boolean {
    const ws = atoms.workspace();
    const tabId = atoms.activeTabId();
    return ws?.pinnedtabids?.includes(tabId) ?? false;
}

function simpleCloseStaticTab() {
    debugLog("simpleCloseStaticTab called");
    const ws = atoms.workspace();
    const tabId = atoms.activeTabId();
    WorkspaceService.CloseTab(ws.oid, tabId).catch((e) => {
        console.error("[closeTab] failed:", e);
    });
    deleteLayoutModelForTab(tabId);
}

function uxCloseBlock(blockId: string) {
    if (isStaticTabPinned() && getStaticTabBlockCount() === 1) {
        TabBarModel.getInstance().jiggleActivePinnedTab();
        return;
    }

    const layoutModel = getLayoutModelForStaticTab();
    const node = layoutModel.getNodeByBlockId(blockId);
    if (node) {
        fireAndForget(() => layoutModel.closeNode(node.id));
    }
}

function genericClose() {
    debugLog("genericClose called");
    if (isStaticTabPinned() && getStaticTabBlockCount() === 1) {
        TabBarModel.getInstance().jiggleActivePinnedTab();
        return;
    }
    const blockCount = getStaticTabBlockCount();
    debugLog("genericClose blockCount", blockCount);
    if (blockCount === 0) {
        debugLog("genericClose calling simpleCloseStaticTab because blockCount is 0");
        simpleCloseStaticTab();
        return;
    }
    debugLog("genericClose calling closeFocusedNode");
    const layoutModel = getLayoutModelForStaticTab();
    fireAndForget(layoutModel.closeFocusedNode.bind(layoutModel));
}

function switchBlockByBlockNum(index: number) {
    const layoutModel = getLayoutModelForStaticTab();
    if (!layoutModel) {
        return;
    }
    layoutModel.switchNodeFocusByBlockNum(index);
    setTimeout(() => {
        globalRefocus();
    }, 10);
}

function cyclePaneFocus(direction: "forward" | "backward") {
    const layoutModel = getLayoutModelForStaticTab();
    const spiralOrder = layoutModel.spiralLeafOrder?.() ?? [];
    if (spiralOrder.length <= 1) return;

    const focusedNode = layoutModel.focusedNode?.();
    const currentIndex = spiralOrder.findIndex((entry) => entry.nodeid === focusedNode?.id);

    let nextIndex: number;
    if (direction === "forward") {
        nextIndex = (currentIndex + 1) % spiralOrder.length;
    } else {
        nextIndex = (currentIndex - 1 + spiralOrder.length) % spiralOrder.length;
    }

    const nextEntry = spiralOrder[nextIndex];
    layoutModel.focusNode(nextEntry.nodeid);
    setTimeout(() => globalRefocus(), 10);
}

function switchBlockInDirection(direction: NavigateDirection) {
    const layoutModel = getLayoutModelForStaticTab();
    layoutModel.switchNodeFocusInDirection(direction);
    setTimeout(() => {
        globalRefocus();
    }, 10);
}

function getAllTabs(ws: Workspace): string[] {
    return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
}

function switchTabAbs(index: number) {
    console.log("switchTabAbs", index);
    const ws = atoms.workspace();
    const newTabIdx = index - 1;
    const tabids = getAllTabs(ws);
    if (newTabIdx < 0 || newTabIdx >= tabids.length) {
        return;
    }
    const newActiveTabId = tabids[newTabIdx];
    setActiveTab(newActiveTabId);
}

function switchTab(offset: number) {
    console.log("switchTab", offset);
    const ws = atoms.workspace();
    const curTabId = atoms.activeTabId();
    let tabIdx = -1;
    const tabids = getAllTabs(ws);
    for (let i = 0; i < tabids.length; i++) {
        if (tabids[i] == curTabId) {
            tabIdx = i;
            break;
        }
    }
    if (tabIdx == -1) {
        return;
    }
    const newTabIdx = (tabIdx + offset + tabids.length) % tabids.length;
    const newActiveTabId = tabids[newTabIdx];
    setActiveTab(newActiveTabId);
}

function handleCmdI() {
    globalRefocus();
}

function globalRefocusWithTimeout(timeoutVal: number) {
    setTimeout(() => {
        globalRefocus();
    }, timeoutVal);
}

function globalRefocus() {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    if (focusedNode == null) {
        // focus a node
        layoutModel.focusFirstNode();
        return;
    }
    const blockId = focusedNode?.data?.blockId;
    if (blockId == null) {
        return;
    }
    refocusNode(blockId);
}

function getDefaultNewBlockDef(): BlockDef {
    const adnbAtom = getSettingsKeyAtom("app:defaultnewblock");
    const adnb = adnbAtom() ?? "term";
    if (adnb == "launcher") {
        return {
            meta: {
                view: "launcher",
            },
        };
    }
    // "term", blank, anything else, fall back to terminal
    const termBlockDef: BlockDef = {
        meta: {
            view: "term",
            controller: "shell",
        },
    };
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    if (focusedNode != null) {
        const blockAtom = WOS.getWaveObjectAtom<Block>(WOS.makeORef("block", focusedNode.data?.blockId));
        const blockData = blockAtom();
        if (blockData?.meta?.view == "term") {
            if (blockData?.meta?.["cmd:cwd"] != null) {
                termBlockDef.meta["cmd:cwd"] = blockData.meta["cmd:cwd"];
            }
        }
        if (blockData?.meta?.connection != null) {
            termBlockDef.meta.connection = blockData.meta.connection;
        }
    }
    return termBlockDef;
}

async function handleCmdN() {
    const blockDef = getDefaultNewBlockDef();
    await createBlock(blockDef);
}

async function handleSplitHorizontal(position: "before" | "after") {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    if (focusedNode == null) {
        return;
    }
    const blockDef = getDefaultNewBlockDef();
    await createBlockSplitHorizontally(blockDef, focusedNode.data.blockId, position);
}

async function handleSplitVertical(position: "before" | "after") {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    if (focusedNode == null) {
        return;
    }
    const blockDef = getDefaultNewBlockDef();
    await createBlockSplitVertically(blockDef, focusedNode.data.blockId, position);
}

let lastHandledEvent: KeyboardEvent | null = null;

// returns [keymatch, T]
function checkKeyMap<T>(waveEvent: WaveKeyboardEvent, keyMap: Map<string, T>): [string, T] {
    for (const key of keyMap.keys()) {
        if (keyutil.checkKeyPressed(waveEvent, key)) {
            const val = keyMap.get(key);
            return [key, val];
        }
    }
    return [null, null];
}

function appHandleKeyDown(waveEvent: WaveKeyboardEvent): boolean {
    if (globalKeybindingsDisabled) {
        return false;
    }
    const nativeEvent = (waveEvent as any).nativeEvent;
    if (lastHandledEvent != null && nativeEvent != null && lastHandledEvent === nativeEvent) {
        console.log("lastHandledEvent return false");
        return false;
    }
    lastHandledEvent = nativeEvent;
    if (activeChord) {
        console.log("handle activeChord", activeChord);
        // If we're in chord mode, look for the second key.
        const chordBindings = globalChordMap.get(activeChord);
        const [, handler] = checkKeyMap(waveEvent, chordBindings);
        if (handler) {
            resetChord();
            return handler(waveEvent);
        } else {
            // invalid chord; reset state and consume key
            resetChord();
            return true;
        }
    }
    const [chordKeyMatch] = checkKeyMap(waveEvent, globalChordMap);
    if (chordKeyMatch) {
        setActiveChord(chordKeyMatch);
        return true;
    }

    const [, globalHandler] = checkKeyMap(waveEvent, globalKeyMap);
    if (globalHandler) {
        const handled = globalHandler(waveEvent);
        if (handled) {
            return true;
        }
    }
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    const blockId = focusedNode?.data?.blockId;
    if (blockId != null && shouldDispatchToBlock(waveEvent)) {
        const bcm = getBlockComponentModel(blockId);
        const viewModel = bcm?.viewModel;
        if (viewModel?.keyDownHandler) {
            const handledByBlock = viewModel.keyDownHandler(waveEvent);
            if (handledByBlock) {
                return true;
            }
        }
    }
    return false;
}

function registerControlShiftStateUpdateHandler() {
    getApi().onControlShiftStateUpdate((state: boolean) => {
        if (state) {
            setControlShift();
        } else {
            unsetControlShift();
        }
    });
}

function tryReinjectKey(event: WaveKeyboardEvent): boolean {
    return appHandleKeyDown(event);
}

function countTermBlocks(): number {
    const allBCMs = getAllBlockComponentModels();
    let count = 0;
    for (const bcm of allBCMs) {
        const viewModel = bcm.viewModel;
        if (viewModel.viewType == "term" && viewModel.isBasicTerm?.()) {
            count++;
        }
    }
    return count;
}

function registerGlobalKeys() {
    globalKeyMap.set("Cmd:]", () => {
        switchTab(1);
        return true;
    });
    globalKeyMap.set("Shift:Cmd:]", () => {
        switchTab(1);
        return true;
    });
    globalKeyMap.set("Cmd:[", () => {
        switchTab(-1);
        return true;
    });
    globalKeyMap.set("Shift:Cmd:[", () => {
        switchTab(-1);
        return true;
    });
    globalKeyMap.set("Cmd:n", () => {
        handleCmdN();
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:n", () => {
        getApi().openNewWindow().catch((e: unknown) => {
            console.error("[keymodel] Failed to open new window:", e);
        });
        return true;
    });
    globalKeyMap.set("Cmd:d", () => {
        handleSplitHorizontal("after");
        return true;
    });
    globalKeyMap.set("Shift:Cmd:d", () => {
        handleSplitVertical("after");
        return true;
    });
    globalKeyMap.set("Cmd:i", () => {
        handleCmdI();
        return true;
    });
    globalKeyMap.set("Cmd:t", () => {
        createTab();
        return true;
    });
    globalKeyMap.set("Cmd:w", () => {
        genericClose();
        return true;
    });
    globalKeyMap.set("Cmd:Shift:w", () => {
        if (isStaticTabPinned()) {
            TabBarModel.getInstance().jiggleActivePinnedTab();
            return true;
        }
        simpleCloseStaticTab();
        return true;
    });
    globalKeyMap.set("Cmd:m", () => {
        const layoutModel = getLayoutModelForStaticTab();
        const focusedNode = layoutModel.focusedNode?.();
        if (focusedNode != null) {
            layoutModel.magnifyNodeToggle(focusedNode.id);
        }
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:ArrowUp", () => {
        switchBlockInDirection(NavigateDirection.Up);
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:ArrowDown", () => {
        switchBlockInDirection(NavigateDirection.Down);
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:ArrowLeft", () => {
        switchBlockInDirection(NavigateDirection.Left);
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:ArrowRight", () => {
        switchBlockInDirection(NavigateDirection.Right);
        return true;
    });
    globalKeyMap.set("Ctrl:]", () => {
        cyclePaneFocus("forward");
        return true;
    });
    globalKeyMap.set("Ctrl:[", () => {
        cyclePaneFocus("backward");
        return true;
    });
    globalKeyMap.set("Ctrl:Shift:k", () => {
        const blockId = getFocusedBlockId();
        if (blockId == null) {
            return true;
        }
        replaceBlock(
            blockId,
            {
                meta: {
                    view: "launcher",
                },
            },
            true
        );
        return true;
    });
    globalKeyMap.set("Cmd:g", () => {
        const bcm = getBlockComponentModel(getFocusedBlockInStaticTab());
        if (bcm?.openSwitchConnection != null) {
            bcm.openSwitchConnection();
            return true;
        }
    });
    globalKeyMap.set("Ctrl:Shift:m", () => {
        const curMI = atoms.isTermMultiInput();
        if (!curMI && countTermBlocks() <= 1) {
            // don't turn on multi-input unless there are 2 or more basic term blocks
            return true;
        }
        setIsTermMultiInput(!curMI);
        return true;
    });
    for (let idx = 1; idx <= 9; idx++) {
        globalKeyMap.set(`Cmd:${idx}`, () => {
            switchTabAbs(idx);
            return true;
        });
        globalKeyMap.set(`Ctrl:Shift:c{Digit${idx}}`, () => {
            switchBlockByBlockNum(idx);
            return true;
        });
        globalKeyMap.set(`Ctrl:Shift:c{Numpad${idx}}`, () => {
            switchBlockByBlockNum(idx);
            return true;
        });
    }
    function activateSearch(event: WaveKeyboardEvent): boolean {
        const bcm = getBlockComponentModel(getFocusedBlockInStaticTab());
        if (bcm == null) return false;
        // Ctrl+f is reserved in most shells
        if (event.control && bcm.viewModel.viewType == "term") {
            return false;
        }
        if (bcm.viewModel.searchAtoms) {
            bcm.viewModel.searchAtoms.isOpen._set(true);
            return true;
        }
        return false;
    }
    function deactivateSearch(): boolean {
        const bcm = getBlockComponentModel(getFocusedBlockInStaticTab());
        if (bcm == null) return false;
        if (bcm.viewModel.searchAtoms && bcm.viewModel.searchAtoms.isOpen()) {
            bcm.viewModel.searchAtoms.isOpen._set(false);
            return true;
        }
        return false;
    }
    globalKeyMap.set("Cmd:f", activateSearch);
    globalKeyMap.set("Escape", () => {
        if (modalsModel.hasOpenModals()) {
            modalsModel.popModal();
            return true;
        }
        if (deactivateSearch()) {
            return true;
        }
        return false;
    });
    // Zoom controls - macOS
    globalKeyMap.set("Cmd:=", () => {
        zoomIn(globalStore);
        return true;
    });
    globalKeyMap.set("Cmd:+", () => {
        zoomIn(globalStore);
        return true;
    });
    globalKeyMap.set("Cmd:-", () => {
        zoomOut(globalStore);
        return true;
    });
    globalKeyMap.set("Cmd:0", () => {
        zoomReset(globalStore);
        return true;
    });

    // Zoom controls - Linux/Windows
    globalKeyMap.set("Ctrl:=", () => {
        zoomIn(globalStore);
        return true;
    });
    globalKeyMap.set("Ctrl:+", () => {
        zoomIn(globalStore);
        return true;
    });
    globalKeyMap.set("Ctrl:-", () => {
        zoomOut(globalStore);
        return true;
    });
    globalKeyMap.set("Ctrl:0", () => {
        zoomReset(globalStore);
        return true;
    });

    const splitBlockKeys = new Map<string, KeyHandler>();
    splitBlockKeys.set("ArrowUp", () => {
        handleSplitVertical("before");
        return true;
    });
    splitBlockKeys.set("ArrowDown", () => {
        handleSplitVertical("after");
        return true;
    });
    splitBlockKeys.set("ArrowLeft", () => {
        handleSplitHorizontal("before");
        return true;
    });
    splitBlockKeys.set("ArrowRight", () => {
        handleSplitHorizontal("after");
        return true;
    });
    globalChordMap.set("Ctrl:Shift:s", splitBlockKeys);
}

function getAllGlobalKeyBindings(): string[] {
    const allKeys = Array.from(globalKeyMap.keys());
    return allKeys;
}

export {
    appHandleKeyDown,
    disableGlobalKeybindings,
    enableGlobalKeybindings,
    getSimpleControlShiftAtom,
    globalRefocus,
    globalRefocusWithTimeout,
    registerControlShiftStateUpdateHandler,
    registerGlobalKeys,
    tryReinjectKey,
    unsetControlShift,
    uxCloseBlock,
};
