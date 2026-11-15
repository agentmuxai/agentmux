// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which panes have their backdrop picker open: one flag per block, read by the frame, set by the pane's menu entry. */

import { createSignal, type Accessor, type Setter } from "solid-js";

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
