// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What each agent pane's Messages panel holds that the person has not seen, for the places that name the pane but
 * do not contain it: its Swarm row and its tab (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md
 * §5.6, second step). The panel counts from its own document nodes and publishes the result here, so the chip
 * everywhere says the same number as the header button, and a pane whose panel is off shows no chip. A pane
 * stays mounted while its tab is in the background, so the count follows it there too.
 */

import { createSignal } from "solid-js";

export interface MessagesUnread {
    unread: number;
    /** Something waits on the person (a sensitive jekt that needs their word). */
    needsYou: boolean;
}

const [byBlock, setByBlock] = createSignal<Readonly<Record<string, MessagesUnread>>>({});

/** The pane's unread count, or null when the pane has nothing unread (reactive). */
export function messagesUnreadFor(blockId: string): MessagesUnread | null {
    const held = byBlock()[blockId];
    return held && held.unread > 0 ? held : null;
}

/** The panel says what it holds; `null` takes the pane's entry away (the panel is off, or the pane closed). */
export function publishMessagesUnread(blockId: string, value: MessagesUnread | null): void {
    const prev = byBlock()[blockId];
    if (!value) {
        if (!prev) return;
        const { [blockId]: _gone, ...rest } = byBlock();
        setByBlock(rest);
        return;
    }
    if (prev && prev.unread === value.unread && prev.needsYou === value.needsYou) return;
    setByBlock({ ...byBlock(), [blockId]: value });
}

/** Tests only. */
export function resetMessagesUnread(): void {
    setByBlock({});
}
