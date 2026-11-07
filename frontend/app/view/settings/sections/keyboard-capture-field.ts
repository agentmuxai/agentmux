// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A field that records shortcuts (the recorder, search by key): while one is mounted the global keys are off, so a
// bound key is recorded instead of run, and the field gets every press in the capture phase, before anything else.

import { onCleanup, onMount } from "solid-js";

import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";

/** How many capture fields are on screen: the global keys come back only when the last one is gone, since
 *  `disableGlobalKeybindings` is one switch and several lines can have a recorder open. */
let open = 0;

/** Wires `onKey` to `field`'s presses, takes the focus, and keeps the global keys off while any field is here. */
export function useKeyCapture(field: () => HTMLInputElement | undefined, onKey: (e: KeyboardEvent) => void): void {
    onMount(() => {
        if (open++ === 0) disableGlobalKeybindings();
        field()?.addEventListener("keydown", onKey, true);
        field()?.focus();
    });
    onCleanup(() => {
        field()?.removeEventListener("keydown", onKey, true);
        if (--open === 0) enableGlobalKeybindings();
    });
}
