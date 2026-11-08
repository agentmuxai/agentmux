// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { hostHas } from "@/util/host-caps";
import { countBasicTerms } from "@/app/store/view-actions";
import { atoms, settingsAtom, setIsTermMultiInput } from "@/app/store/global";
import { createTab } from "@/app/store/tab-actions";
import { getBlockComponentModel, getFocusedBlockId, openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { replaceBlock } from "@/app/store/block-layout-actions";
import { zoomAllPanesReset, zoomIn, zoomOut, zoomReset } from "@/app/store/zoom";
import { requestTabRename } from "@/app/tab/tab-rename-request";
import { requestComposerFocus } from "@/app/agent-model/composer-focus";
import { resizeFocusedInDirection, swapFocusedInDirection } from "@/layout/lib/layoutKeyboard";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { modalsModel } from "./modalmodel";
import { openCommandPalette } from "@/app/store/open-command-palette";
import { ReplacePaneConfirm } from "@/app/modals/replace-pane-confirm";
import { openPaneModal, paneRequests } from "@/app/store/pane-requests";
import { handleCmdN, handleSplitHorizontal, handleSplitVertical } from "./keymodel-blockcreate";
import { type KeyHandler, keyCommands } from "./keymodel-dispatch";
import { clearPaneTabSelection } from "@/app/element/pane-tab-selection";
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
import { setUserKeybindings } from "@/app/keybindings/registry";
import { registerHostKeysPush } from "./host-keys-push";
import { createEffect, createRoot } from "solid-js";
import { commandRegistry } from "./command-registry";
import { openNewWindowFromHere } from "./window-appearance";

function countTermBlocks(): number {
    return countBasicTerms();
}

const on = (command: string, handler: KeyHandler) => keyCommands.set(command, handler);
const run = (fn: () => void): KeyHandler => () => {
    fn();
    return true;
};

/** General. */
function registerGeneralKeys() {
    on("view:command-palette", run(() => void openCommandPalette()));
    on("app:settings", run(() => void openOrFocusPaneByView("settings")));
    on("help:shortcuts", run(() => void openOrFocusPaneByView("help")));
    on("app:escape", () => {
        if (modalsModel.hasOpenModals()) {
            modalsModel.closeTopModal();
            return true;
        }
        // One Escape does one thing: a modal, then an active search, then picked pane tabs
        // (Ctrl/Cmd+click), then (not here) un-magnify. A handled key goes no further.
        if (deactivateSearch()) return true;
        return clearPaneTabSelection();
    });
}

/** Tabs and windows. */
function registerTabKeys() {
    // Declined (false) on a host without multiWindow, so the key goes on to the terminal.
    on("window:new", () => {
        if (!hostHas("multiWindow")) return false;
        openNewWindowFromHere().catch((e: unknown) => console.error("[keymodel] Failed to open new window:", e));
        return true;
    });
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
}

/** Panes: create, close, move focus, swap and resize. */
function registerPaneLayoutKeys() {
    on("pane:new", run(() => handleCmdN()));
    on("split:right", run(() => handleSplitHorizontal("after")));
    on("split:down", run(() => handleSplitVertical("after")));
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
}

const REPLACE_PANE_SOURCE = "replace-pane";

/** Panes: composer focus, replace, connection, multi-input, voice. */
function registerPaneActionKeys() {
    on("agent:focusComposer", () => {
        const blockId = getFocusedBlockId();
        const vm = blockId != null ? getBlockComponentModel(blockId)?.viewModel : undefined;
        if (blockId == null || vm?.viewType !== "agent") return false;
        // The mounted composer takes focus directly; a request only queues it
        // for a footer that hasn't mounted yet.
        if (!vm.giveFocus?.()) requestComposerFocus(blockId);
        return true;
    });
    on("pane:replaceWithLauncher", (e) => {
        const blockId = getFocusedBlockId();
        if (blockId == null) {
            return true;
        }
        if (getBlockComponentModel(blockId)?.viewModel?.viewType === "shortcuts") {
            return true;
        }
        // One confirmation at a time: a held key repeats, and global
        // shortcuts still run while a modal is open.
        if (e.repeat || paneRequests.of(blockId).some((r) => r.source === REPLACE_PANE_SOURCE)) {
            return true;
        }
        // Asked in that pane only; nothing waits on it, the person just pressed the key.
        openPaneModal(
            blockId,
            ReplacePaneConfirm,
            { onConfirm: () => replaceBlock(blockId, { meta: { view: "shortcuts" } }, true) },
            { source: REPLACE_PANE_SOURCE, waits: false }
        );
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
}

/** Find, zoom and the terminal's own keys. */
function registerFindAndZoomKeys() {
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

/** What each command in the shortcut table does (keybindings/defaults.ts).
 *  The keys themselves live in the table, so the help pane, the menus and
 *  the docs show what actually runs. A handler returns false when it didn't
 *  apply, which leaves the key to whatever else wants it. */
function registerGlobalKeys() {
    registerGeneralKeys();
    registerTabKeys();
    registerPaneLayoutKeys();
    registerPaneActionKeys();
    registerFindAndZoomKeys();
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

/** Applies the `keybindings` setting now and whenever it changes; an entry
 *  that can't be used is skipped with a console warning (report §6.6). A focused
 *  browser pane follows it too: the host is sent the keys to forward. */
function registerUserKeybindings(): void {
    registerHostKeysPush();
    createRoot(() =>
        createEffect(() => {
            const known = (id: string) => keyCommands.has(id) || commandRegistry.get(id) != null;
            for (const message of setUserKeybindings(settingsAtom()?.["keybindings"], known)) {
                console.warn(`[keybindings] ${message}`);
            }
        })
    );
}

export { registerGlobalKeys, registerUserKeybindings };

export {
    appHandleKeyDown,
    disableGlobalKeybindings,
    enableGlobalKeybindings,
    installShortcutApi,
    keyboardMouseDownHandler,
    registerControlShiftTracking,
    registerHostShortcuts,
    registerChordCapture,
} from "./keymodel-dispatch";

export { globalRefocus, globalRefocusWithTimeout } from "./keymodel-nav";
