// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, getBlockComponentModel, setControlShiftDelayAtom } from "@/app/store/global";
import { getLayoutModelForStaticTab } from "@/layout/index";
import { isEditableTarget } from "@/util/focusutil";
import * as keyutil from "@/util/keyutil";
import { CHORD_TIMEOUT } from "@/util/sharedconst";
import { createSignal } from "solid-js";

export type KeyHandler = (event: MuxKeyboardEvent) => boolean;

const [simpleControlShift, setSimpleControlShift] = createSignal(false);
export const globalKeyMap = new Map<string, (muxEvent: MuxKeyboardEvent) => boolean>();
export const globalChordMap = new Map<string, Map<string, KeyHandler>>();
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

export function disableGlobalKeybindings() {
    globalKeybindingsDisabled = true;
}

export function enableGlobalKeybindings() {
    globalKeybindingsDisabled = false;
}

function shouldDispatchToBlock(e: MuxKeyboardEvent): boolean {
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

let lastHandledEvent: KeyboardEvent | null = null;

// Global shortcuts that are also text-editing keys. While the user types in
// a text field, the composer or the code editor, these keep their editing
// meaning (word/line selection, indent, CodeMirror's delete-line) instead
// of moving focus or replacing the pane
// (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §3.3).
const TEXT_EDITING_KEYS = [
    "Ctrl:Shift:ArrowLeft",
    "Ctrl:Shift:ArrowRight",
    "Ctrl:Shift:ArrowUp",
    "Ctrl:Shift:ArrowDown",
    "Ctrl:[",
    "Ctrl:]",
    "Ctrl:Shift:k",
];

/** Focus is somewhere the user types. Not the terminal: its hidden textarea
 *  is focus plumbing, and the terminal decides which keys go to the shell
 *  itself (term-shell-keys.ts). */
export function isTypingFocus(el: Element | null = document.activeElement): boolean {
    if (!(el instanceof HTMLElement)) return false;
    if (el.classList.contains("xterm-helper-textarea")) return false;
    if (el.classList.contains("dummy-focus") || el.classList.contains("dummy")) return false;
    return isEditableTarget(el);
}

// returns [keymatch, T]
function checkKeyMap<T>(muxEvent: MuxKeyboardEvent, keyMap: Map<string, T>): [string, T] {
    for (const key of keyMap.keys()) {
        if (keyutil.checkKeyPressed(muxEvent, key)) {
            const val = keyMap.get(key);
            return [key, val];
        }
    }
    return [null, null];
}

export function appHandleKeyDown(muxEvent: MuxKeyboardEvent): boolean {
    if (globalKeybindingsDisabled) {
        return false;
    }
    const nativeEvent = (muxEvent as any).nativeEvent;
    // A pane or the code editor already handled this key (it called
    // preventDefault). Solid's delegated onKeyDown and this handler both
    // listen on `document`, so a pane's stopPropagation can't stop us; this
    // check is what keeps one key press from also running a global shortcut.
    if (nativeEvent?.defaultPrevented) {
        return false;
    }
    if (lastHandledEvent != null && nativeEvent != null && lastHandledEvent === nativeEvent) {
        console.log("lastHandledEvent return false");
        return false;
    }
    lastHandledEvent = nativeEvent;
    if (activeChord) {
        console.log("handle activeChord", activeChord);
        // If we're in chord mode, look for the second key.
        const chordBindings = globalChordMap.get(activeChord);
        const [, handler] = checkKeyMap(muxEvent, chordBindings);
        if (handler) {
            resetChord();
            return handler(muxEvent);
        } else {
            // invalid chord; reset state and consume key
            resetChord();
            return true;
        }
    }
    if (isTypingFocus() && TEXT_EDITING_KEYS.some((k) => keyutil.checkKeyPressed(muxEvent, k))) {
        return false;
    }
    const [chordKeyMatch] = checkKeyMap(muxEvent, globalChordMap);
    if (chordKeyMatch) {
        setActiveChord(chordKeyMatch);
        return true;
    }

    const [, globalHandler] = checkKeyMap(muxEvent, globalKeyMap);
    if (globalHandler) {
        const handled = globalHandler(muxEvent);
        if (handled) {
            return true;
        }
    }
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    const blockId = focusedNode?.data?.blockId;
    if (blockId != null && shouldDispatchToBlock(muxEvent)) {
        const bcm = getBlockComponentModel(blockId);
        const viewModel = bcm?.viewModel;
        if (viewModel?.keyDownHandler) {
            const handledByBlock = viewModel.keyDownHandler(muxEvent);
            if (handledByBlock) {
                return true;
            }
        }
    }
    return false;
}

export function registerControlShiftStateUpdateHandler() {
    getApi().onControlShiftStateUpdate((state: boolean) => {
        if (state) {
            setControlShift();
        } else {
            unsetControlShift();
        }
    });
}

