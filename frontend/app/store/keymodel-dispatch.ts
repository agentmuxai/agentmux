// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, getBlockComponentModel, setControlShiftDelayAtom } from "@/app/store/global";
import { getLayoutModelForStaticTab } from "@/layout/index";
import * as keyutil from "@/util/keyutil";
import { CHORD_TIMEOUT } from "@/util/sharedconst";
import { createSignal } from "solid-js";

export type KeyHandler = (event: WaveKeyboardEvent) => boolean;

const [simpleControlShift, setSimpleControlShift] = createSignal(false);
export const globalKeyMap = new Map<string, (waveEvent: WaveKeyboardEvent) => boolean>();
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

export function appHandleKeyDown(waveEvent: WaveKeyboardEvent): boolean {
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

export function registerControlShiftStateUpdateHandler() {
    getApi().onControlShiftStateUpdate((state: boolean) => {
        if (state) {
            setControlShift();
        } else {
            unsetControlShift();
        }
    });
}

