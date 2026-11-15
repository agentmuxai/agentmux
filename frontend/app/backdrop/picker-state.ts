// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which panes have their backdrop picker open: one flag per block, read by the frame, set by the pane's menu entry. */

import { createSignal, onCleanup, onMount, type Accessor, type Setter } from "solid-js";

const flags = new Map<string, [Accessor<boolean>, Setter<boolean>]>();

function flag(blockId: string) {
    let f = flags.get(blockId);
    if (!f) {
        f = createSignal(false);
        flags.set(blockId, f);
    }
    return f;
}

/** Whether a block's picker is open, as a signal. Safe to read before it was ever opened. */
export const isBackdropPickerOpen = (blockId: string): Accessor<boolean> => flag(blockId)[0];

export const openBackdropPicker = (blockId: string): void => void flag(blockId)[1](true);
export const closeBackdropPicker = (blockId: string): void => void flag(blockId)[1](false);

/** Forget a block that is gone. */
export function releaseBackdropPicker(blockId: string): void {
    flags.get(blockId)?.[1](false);
    flags.delete(blockId);
}

/**
 * The picker's keyboard hold on its pane: the panel takes focus when it opens (so Esc works at once, and a screen
 * reader is placed in it), and Esc closes it from anywhere inside its own pane. A swatch click can hand focus on to
 * the pane's composer or terminal; Esc must still reach the picker there. An Esc another handler already used
 * (`defaultPrevented`) is left to it, and one pressed in another pane is none of this picker's business.
 */
export function useDialogKeys(panel: () => HTMLElement | undefined, close: () => void): void {
    onMount(() => panel()?.focus());
    const onKey = (e: KeyboardEvent) => {
        if (e.key !== "Escape" || e.defaultPrevented) return;
        const frame = panel()?.closest(".block-frame-default");
        if (frame && e.target instanceof Node && frame.contains(e.target)) close();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
}
