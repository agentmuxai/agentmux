// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getVoiceSession } from "@/app/hook/useVoiceInput";
import {
    atoms,
    createTab,
    getAllBlockComponentModels,
    getApi,
    getBlockComponentModel,
    getFocusedBlockId,
    replaceBlock,
    setIsTermMultiInput,
} from "@/app/store/global";
import { zoomIn, zoomOut, zoomReset } from "@/app/store/zoom.platform";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { modalsModel, openModal } from "./modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import { handleCmdN, handleSplitHorizontal, handleSplitVertical } from "./keymodel-blockcreate";
import { type KeyHandler, globalChordMap, globalKeyMap } from "./keymodel-dispatch";
import {
    cyclePaneFocus,
    genericClose,
    getFocusedBlockInStaticTab,
    handleCmdI,
    simpleCloseStaticTab,
    switchBlockByBlockNum,
    switchBlockInDirection,
    switchTab,
    switchTabAbs,
} from "./keymodel-nav";

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
    // Ctrl+Shift+V — toggle voice input on the currently focused pane.
    // Mirrors MicButton click semantics: bind target first, then start
    // OR stop OR retarget. No-op on panes whose ViewModel doesn't
    // expose voiceHandle (e.g. browser, editor). Spec:
    // docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §6.
    globalKeyMap.set("Ctrl:Shift:v", () => {
        const blockId = getFocusedBlockInStaticTab();
        if (!blockId) return true;
        const bcm = getBlockComponentModel(blockId);
        const vm: any = bcm?.viewModel;
        if (!vm?.voiceHandle) {
            // No-op on non-supporting panes. (Could surface a toast in Phase 3.)
            return true;
        }
        const voice = getVoiceSession();
        if (!voice.isAvailable()) return true;
        const wasListening = voice.isListening();
        const wasMine = wasListening && voice.currentTargetId() === blockId;
        voice.registerPane(blockId, vm.voiceHandle());
        if (!wasListening) {
            voice.toggleListening();
        } else if (wasMine) {
            voice.toggleListening();
        }
        // else: retarget without toggle (same logic as MicButton click)
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
            modalsModel.closeTopModal();
            return true;
        }
        if (deactivateSearch()) {
            return true;
        }
        return false;
    });
    // Zoom controls - macOS
    globalKeyMap.set("Cmd:=", () => {
        zoomIn();
        return true;
    });
    globalKeyMap.set("Cmd:+", () => {
        zoomIn();
        return true;
    });
    globalKeyMap.set("Cmd:-", () => {
        zoomOut();
        return true;
    });
    globalKeyMap.set("Cmd:0", () => {
        zoomReset();
        return true;
    });

    // Zoom controls - Linux/Windows
    globalKeyMap.set("Ctrl:=", () => {
        zoomIn();
        return true;
    });
    globalKeyMap.set("Ctrl:+", () => {
        zoomIn();
        return true;
    });
    globalKeyMap.set("Ctrl:-", () => {
        zoomOut();
        return true;
    });
    globalKeyMap.set("Ctrl:0", () => {
        zoomReset();
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

    globalKeyMap.set("Ctrl:p", () => {
        openModal(CommandPaletteModal);
        return true;
    });
}

function getAllGlobalKeyBindings(): string[] {
    const allKeys = Array.from(globalKeyMap.keys());
    return allKeys;
}

export { registerGlobalKeys };

export {
    appHandleKeyDown,
    disableGlobalKeybindings,
    enableGlobalKeybindings,
    keyboardMouseDownHandler,
    registerControlShiftStateUpdateHandler,
} from "./keymodel-dispatch";

export { globalRefocus, globalRefocusWithTimeout } from "./keymodel-nav";
