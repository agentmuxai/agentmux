// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { Pill } from "@/app/element/ui/Pill";
import { messagesUnreadFor } from "@/app/store/messages-unread";
import { badgeText } from "@/app/view/agent/messages/messages-select";

/** What the chip says when hovered: the same words as the pane's Messages button. */
export const chipTitle = (unread: number, needsYou: boolean): string =>
    `Messages, ${unread} unread${needsYou ? ", needs you" : ""}`;

/**
 * The count of messages the person has not seen in a pane's Messages panel, on the pane's Swarm row and its tab: the
 * same number the pane's header button shows (store/messages-unread.ts). Nothing while there are none.
 */
export function MessagesUnreadChip(props: { blockId: string }): JSX.Element {
    const held = () => messagesUnreadFor(props.blockId);
    return (
        <Show when={held()}>
            {(h) => (
                <span class="messages-unread-chip" role="img" aria-label={chipTitle(h().unread, h().needsYou)}>
                    <Pill tone="accent" title={chipTitle(h().unread, h().needsYou)}>
                        {badgeText(h().unread)}
                    </Pill>
                </span>
            )}
        </Show>
    );
}
