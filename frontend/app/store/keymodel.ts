// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { basicTermModels } from "@/app/view/term/term-models";
import {
    atoms,
    createTab,
    getApi,
    getBlockComponentModel,
    getFocusedBlockId,
    openOrFocusPaneByView,
    replaceBlock,
    setIsTermMultiInput,
} from "@/app/store/global";
import { zoomAllPanesReset, zoomIn, zoomOut, zoomReset } from "@/app/store/zoom";
import { requestTabRename } from "@/app/tab/tab-rename-request";
import { requestComposerFocus } from "@/app/view/agent/composer-focus";
import { resizeFocusedInDirection, swapFocusedInDirection } from "@/layout/lib/layoutKeyboard";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { modalsModel, openModal } from "./modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import { ReplacePaneConfirm } from "@/app/modals/replace-pane-confirm";
import { handleCmdN, handleSplitHorizontal, handleSplitVertical } from "./keymodel-blockcreate";
import { type KeyHandler, keyCommands } from "./keymodel-dispatch";
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
    switchTabLast,
    moveActiveTab,
} from "./keymodel-nav";

function countTermBlocks(): number {
    return basicTermModels().length;
}

/** What each command in the shortcut table does (keybindings/defaults.ts).
 *  The keys themselves live in the table, so the help pane, the menus and
 *  the docs show what actually runs. A handler returns false when it didn't
 *  apply, which leaves the key to whatever else wants it. */
function registerGlobalKeys() {
    const on = (command: string, handler: KeyHandler) => keyCommands.set(command, handler);
    const run = (fn: () => void): KeyHandler => () => {
        fn();
        return true;
    };

    // ── General ──
    on("view:command-palette", run(() => openModal(CommandPaletteModal)));
    on("app:settings", run(() => void openOrFocusPaneByView("settings")));
    on("help:shortcuts", run(() => void openOrFocusPaneByView("help")));
    on("app:escape", () => {
        if (modalsModel.hasOpenModals()) {
            modalsModel.closeTopModal();
            return true;
        }
        return deactivateSearch();
    });

    // ── Tabs & windows ──
    on(
        "window:new",
        run(() =>
            getApi()
                .openNewWindow()
                .catch((e: unknown) => console.error("[keymodel] Failed to open new window:", e))
        )
    );
    on("tab:new", run(() => createTab()));
    on("tab:close", run(() => simpleCloseStaticTab()));
    on("tab:next", run(() => switchTab(1)));
    on("tab:prev", run(() => switchTab(-1)));
    for (let idx = 1; idx <= 8; idx++) {
        on(`tab:goto:${idx}`, run(() => switchTabAbs(idx)));
    }
    on("tab:goto:last", run(() => switchTabLast()));
    on("tab:moveLeft", run(() => moveActiveTab(-1)));
    on("tab:moveRight", run(() => moveActiveTab(1)));
    on("tab:rename", run(() => requestTabRename(atoms.activeTabId())));

    // ── Panes ──
    on("pane:new", run(() => handleCmdN()));
    on("split:right", run(() => handleSplitHorizontal("after")));
    on("split:left", run(() => handleSplitHorizontal("before")));
    on("split:down", run(() => handleSplitVertical("after")));
    on("split:up", run(() => handleSplitVertical("before")));
    on("pane:close", run(() => genericClose()));
    on("pane:magnify", () => {
        const layoutModel = getLayoutModelForStaticTab();
        const focusedNode = layoutModel.focusedNode?.();
        if (focusedNode != null) {
            layoutModel.magnifyNodeToggle(focusedNode.id);
        }
        return true;
    });
    on("pane:focus:up", run(() => switchBlockInDirection(NavigateDirection.Up)));
    on("pane:focus:down", run(() => switchBlockInDirection(NavigateDirection.Down)));
    on("pane:focus:left", run(() => switchBlockInDirection(NavigateDirection.Left)));
    on("pane:focus:right", run(() => switchBlockInDirection(NavigateDirection.Right)));
    on("pane:focus:next", run(() => cyclePaneFocus("forward")));
    on("pane:focus:prev", run(() => cyclePaneFocus("backward")));
    for (let idx = 1; idx <= 9; idx++) {
        on(`pane:focus:${idx}`, run(() => switchBlockByBlockNum(idx)));
    }
    const directions = { up: NavigateDirection.Up, down: NavigateDirection.Down, left: NavigateDirection.Left, right: NavigateDirection.Right };
    for (const [name, dir] of Object.entries(directions)) {
        on(`pane:swap:${name}`, () => swapFocusedInDirection(getLayoutModelForStaticTab(), dir));
        on(`pane:resize:${name}`, () => resizeFocusedInDirection(getLayoutModelForStaticTab(), dir));
    }
    on("pane:refocus", run(() => handleCmdI()));
    on("agent:focusComposer", () => {
        const blockId = getFocusedBlockId();
        if (blockId == null || getBlockComponentModel(blockId)?.viewModel?.viewType !== "agent") return false;
        requestComposerFocus(blockId);
        return true;
    });
    on("pane:replaceWithLauncher", (e) => {
        const blockId = getFocusedBlockId();
        if (blockId == null) {
            return true;
        }
        if (getBlockComponentModel(blockId)?.viewModel?.viewType === "launcher") {
            return true;
        }
        // One confirmation at a time: a held key repeats, and global
        // shortcuts still run while a modal is open.
        if (e.repeat || modalsModel.isModalOpen(ReplacePaneConfirm)) {
            return true;
        }
        modalsModel.openModal(ReplacePaneConfirm, {
            onConfirm: () =>
                replaceBlock(
                    blockId,
                    {
                        meta: {
                            view: "launcher",
                        },
                    },
                    true
                ),
        });
        return true;
    });
    on("pane:changeConnection", () => {
        const bcm = getBlockComponentModel(getFocusedBlockInStaticTab());
        if (bcm?.openSwitchConnection != null) {
            bcm.openSwitchConnection();
            return true;
        }
        return false;
    });
    on("term:multiInput", () => {
        const curMI = atoms.isTermMultiInput();
        if (!curMI && countTermBlocks() <= 1) {
            // don't turn on multi-input unless there are 2 or more basic term blocks
            return true;
        }
        setIsTermMultiInput(!curMI);
        return true;
    });
    // Voice input on the focused pane. Mirrors MicButton click semantics:
    // bind target first, then start OR stop OR retarget. No-op on panes whose
    // ViewModel doesn't expose voiceHandle (e.g. browser, editor). Spec:
    // docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §6.
    on("pane:voice", () => {
        const blockId = getFocusedBlockInStaticTab();
        if (!blockId) return true;
        const bcm = getBlockComponentModel(blockId);
        const vm: any = bcm?.viewModel;
        if (!vm?.voiceHandle) {
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

    // ── Find & zoom ──
    on("pane:find", () => activateSearch());
    on("view:zoom:in", run(() => zoomIn()));
    on("view:zoom:out", run(() => zoomOut()));
    on("view:zoom:reset", run(() => zoomReset()));
    on("view:zoom:resetAll", run(() => zoomAllPanesReset()));

    // The terminal runs its own copy / paste / clear (termViewModel.ts).
    on("term:copy", () => false);
    on("term:paste", () => false);
    on("term:clear", () => false);
}

function activateSearch(): boolean {
    const bcm = getBlockComponentModel(getFocusedBlockInStaticTab());
    if (bcm == null) return false;
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

export { registerGlobalKeys };

export {
    appHandleKeyDown,
    disableGlobalKeybindings,
    enableGlobalKeybindings,
    keyboardMouseDownHandler,
    registerControlShiftTracking,
} from "./keymodel-dispatch";

export { globalRefocus, globalRefocusWithTimeout } from "./keymodel-nav";
