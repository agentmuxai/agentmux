// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages button in the pane's header: how the panel opens and closes, at every pane width. While the panel is
 * on, the button is there, with the unread count and a dot while something waits on the person, so a new message is
 * noticeable with the panel closed.
 */

import { onCleanup, Show, type Accessor, type JSX } from "solid-js";

import { Pill } from "@/app/element/ui/Pill";
import type { SignalAtom } from "@/util/util";
import { makeIconClass } from "@/util/util";
import { badgeText } from "./messages-select";
import { publishMessagesHeaderButton, type MessagesPanelControls } from "./messages-state";

/** The class the header button carries, to put the keyboard back on it when the panel closes. */
export const MESSAGES_TOGGLE_CLASS = "messages-toggle";

/** Keeps the button in a narrow header: with the bar gone it is the only way in (mixins.scss, `block-frame-end-keep`). */
const KEEP_CLASS = "block-frame-end-keep";

/** The button's face: the envelope, the unread count and a dot while something waits on the person. */
function ButtonFace(props: { unread: number; needsYou: boolean }): JSX.Element {
    return (
        <span class="messages-header-button">
            <i class={makeIconClass("envelope", true)} />
            <Show when={props.unread > 0}>
                <Pill tone="accent">{badgeText(props.unread)}</Pill>
            </Show>
            <Show when={props.needsYou}>
                <span class="messages-header-button__needs" role="img" aria-label="Needs you" />
            </Show>
        </span>
    );
}

/** `Messages`, with what it holds: the title and the accessible name of the button. */
export const buttonTitle = (unread: number, needsYou: boolean): string =>
    `Messages${unread ? `, ${unread} unread` : ""}${needsYou ? ", needs you" : ""}`;

/**
 * Put the Messages button in the pane's header, pressed while the panel is open, and take it away when the panel goes
 * (the pane hid it, or the pane closed). One declaration for the panel's life, its title and face reading the
 * count themselves: the header draws a new button for a new object, which would drop the keyboard or the tooltip
 * the moment a message arrived.
 */
export function publishHeaderButton(
    blockId: string,
    controls: MessagesPanelControls,
    unread: Accessor<number>,
    needsYou: Accessor<boolean>
): void {
    // A fake SignalAtom, as the find bar's magnifier uses: the read tracks the panel, the write opens or closes it.
    const active = (() => controls.state().open) as SignalAtom<boolean>;
    active._set = (open: boolean) => controls.patch({ open });
    publishMessagesHeaderButton(blockId, {
        elemtype: "toggleiconbutton",
        // A function, not a node: the pane's header is drawn in more than one place (the tile preview among
        // them), and each draws its own face. One node can only sit in one of them.
        icon: (() => <ButtonFace unread={unread()} needsYou={needsYou()} />) as unknown as JSX.Element,
        get title() {
            return buttonTitle(unread(), needsYou());
        },
        className: `${MESSAGES_TOGGLE_CLASS} ${KEEP_CLASS}`,
        fitContent: true,
        active,
    });
    onCleanup(() => publishMessagesHeaderButton(blockId, null));
}
