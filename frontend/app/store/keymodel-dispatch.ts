// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getBlockComponentModel, setControlShiftDelayAtom } from "@/app/store/global";
import { getLayoutModelForStaticTab } from "@/layout/index";
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
    if (lastHandledEvent != null && nativeEvent != null && lastHandledEvent === nativeEvent) {
        return false;
    }
    lastHandledEvent = nativeEvent;
    if (activeChord) {
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

/** Shows the numbered pane overlay while Ctrl+Shift is held (after a short
 *  delay, so a quick Ctrl+Shift+key chord doesn't flash it). It used to wait
 *  for a `control-shift-state-update` event from the host that nothing ever
 *  sent, so the overlay never appeared. */
export function registerControlShiftTracking() {
    const update = (e: KeyboardEvent) => {
        if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey) {
            if (!simpleControlShift()) setControlShift();
        } else if (simpleControlShift()) {
            unsetControlShift();
        }
    };
    document.addEventListener("keydown", update, true);
    document.addEventListener("keyup", update, true);
    window.addEventListener("blur", () => unsetControlShift());
}

