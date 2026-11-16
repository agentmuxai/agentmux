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

/** Open a block's picker, and close every other pane's: one picker at a time. */
export function openBackdropPicker(blockId: string): void {
    for (const [id, [, set]] of flags) if (id !== blockId) set(false);
    flag(blockId)[1](true);
}
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
 * (`defaultPrevented`) is left to it, and one pressed in another pane is none of this picker's business. The Esc that
 * closes it is consumed, so a handler further out (the layout's, which un-magnifies a pane) does not act on it too.
 * When it closes, focus goes back to where it was.
 */
export function useDialogKeys(panel: () => HTMLElement | undefined, close: () => void): void {
    // Where focus was when the picker opened: given back when it closes, if focus is then in the picker or nowhere.
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
    onMount(() => panel()?.focus());
    onCleanup(() => {
        const now = document.activeElement;
        if (opener?.isConnected && (!now || now === document.body || panel()?.contains(now) || now === panel())) opener.focus();
    });
    const onKey = (e: KeyboardEvent) => {
        if (e.key !== "Escape" || e.defaultPrevented) return;
        const frame = panel()?.closest(".block-frame-default");
        if (!frame || !(e.target instanceof Node) || !frame.contains(e.target)) return;
        // Used here, so used: a magnified pane's own Esc (the layout's, on `window`) must not also un-magnify it.
        e.preventDefault();
        e.stopPropagation();
        close();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
}
