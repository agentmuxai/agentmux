// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the pane a block lives in is minimized (a header chip, its content laid out but
 * hidden). `pane-leaf-chrome.tsx` sets it for every member of the pane's stack; the one
 * visibility signal (`pane-tab-visibility.ts`) reads it, so a minimized pane's members, the
 * front one included, count as dormant rather than as on screen.
 *
 * Kept apart from `block-component-registry.ts`'s dormancy set on purpose: that set also decides
 * which panes the "all panes" consumers (multi-input broadcast, all-panes zoom) see, and a
 * minimized pane stays in those.
 */

import { createSignal, type Accessor } from "solid-js";

const signals = new Map<string, [Accessor<boolean>, (v: boolean) => void]>();

function signalFor(blockId: string): [Accessor<boolean>, (v: boolean) => void] {
    let sig = signals.get(blockId);
    if (!sig) {
        sig = createSignal<boolean>(false);
        signals.set(blockId, sig);
    }
    return sig;
}

/** Reactive: false for a block never marked, and for a pane that is expanded. */
export function isBlockMinimized(blockId: string): Accessor<boolean> {
    return signalFor(blockId)[0];
}

/** Called by the pane's chrome as its minimized state changes, and with false when a member leaves the stack. */
export function setBlockMinimized(blockId: string, minimized: boolean): void {
    // Nothing to record for a block that was never minimized.
    if (!minimized && !signals.has(blockId)) return;
    signalFor(blockId)[1](minimized);
}
