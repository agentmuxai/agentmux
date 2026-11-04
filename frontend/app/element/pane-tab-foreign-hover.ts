// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The hover state of a pane tab strip while a tab from another pane is dragged over its drop zone:
// whether it is hovered, what the window's flash budget gave this entry (drop-blink-clock.ts), and the
// ghost pill's text (what the drop would add, shown where it lands). Kept apart from PaneTabStrip.tsx,
// which only draws it.

import { batch, createSignal, onCleanup } from "solid-js";
import { beginHover, type Hover, type HoverFlash } from "./drop-blink-clock";

export interface ForeignHoverOptions {
    /** The area the hover flashes, to size it against the flash rule (drop-blink-clock.ts). */
    flashArea: () => number | undefined;
    /** The ghost pill's text for a dragged payload: none when the drop would add nothing here. */
    incomingOf: (data: Record<string | symbol, unknown>) => string | null;
}

export function createForeignHover(options: ForeignHoverOptions) {
    const [foreignHover, setForeignHover] = createSignal(false);
    // Set before `foreignHover`, so the class toggles are applied in the same frame.
    const [flash, setFlash] = createSignal<HoverFlash>("blink");
    // What the hovering drag would add to this strip, shown as a ghost pill where it lands (at the end).
    const [incoming, setIncoming] = createSignal<string | null>(null);
    let hover: Hover | null = null;
    const begin = (source?: { data: Record<string | symbol, unknown> }) => {
        hover?.end();
        hover = beginHover(performance.now(), options.flashArea());
        batch(() => {
            setFlash(hover!.kind);
            setIncoming(source ? options.incomingOf(source.data) : null);
            setForeignHover(true);
        });
    };
    const end = () => {
        hover?.end();
        hover = null;
        batch(() => {
            setIncoming(null);
            setForeignHover(false);
        });
    };
    onCleanup(() => hover?.end());
    return { foreignHover, flash, incoming, begin, end, hovering: () => hover !== null };
}
