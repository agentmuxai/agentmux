// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A field that records shortcuts (the recorder, search by key): while it is mounted the global keys are off, so a
// bound key is recorded instead of run, and the field gets every press in the capture phase, before anything else.

import { onCleanup, onMount } from "solid-js";

import { disableGlobalKeybindings, enableGlobalKeybindings } from "@/app/store/keymodel";

/** Wires `onKey` to `field`'s presses, takes the focus, and turns the global keys off until the field is gone. */
export function useKeyCapture(field: () => HTMLInputElement | undefined, onKey: (e: KeyboardEvent) => void): void {
    onMount(() => {
        disableGlobalKeybindings();
        field()?.addEventListener("keydown", onKey, true);
        field()?.focus();
    });
    onCleanup(() => {
        field()?.removeEventListener("keydown", onKey, true);
        enableGlobalKeybindings();
    });
}
