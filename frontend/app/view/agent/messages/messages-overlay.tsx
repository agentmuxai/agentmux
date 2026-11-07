// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages panel in a narrow pane (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5.1):
 * below `NARROW_BELOW_PX` the panel never takes a column from the conversation. It opens over it from a button in
 * the pane's header, like the Kanban's task drawer, and closes back to nothing.
 */

import { createEffect, createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";

import { Pill } from "@/app/element/ui/Pill";
import { makeIconClass } from "@/util/util";
import { badgeText } from "./messages-select";
import { publishMessagesHeaderButton } from "./messages-state";

/** A pane narrower than this (in its own zoomed pixels) gets the overlay form: the conversation keeps its width. */
export const NARROW_BELOW_PX = 640;

/**
 * Whether the pane is narrow, from the width of the element the panel sits in. The width is in unzoomed pixels; the
 * conversation is drawn at the pane's zoom, so the room it has is the width over that zoom. A width of 0 (not laid
 * out, or no ResizeObserver) is unknown and counts as wide.
 */
export function createNarrow(parent: () => HTMLElement | null | undefined, zoom: Accessor<number>): Accessor<boolean> {
    const [width, setWidth] = createSignal(0);
    onMount(() => {
        const el = parent();
        if (!el) return;
        setWidth(el.clientWidth);
        if (typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver((entries) => setWidth(entries[entries.length - 1].contentRect.width));
        observer.observe(el);
        onCleanup(() => observer.disconnect());
    });
    return () => width() > 0 && width() / (zoom() || 1) < NARROW_BELOW_PX;
}

/** The header button's face: the envelope, the unread count and a dot while something waits on the person. */
function ButtonFace(props: { unread: number; needsYou: boolean }): JSX.Element {
    return (
        <span class="messages-header-button">
            <i class={makeIconClass("envelope", true)} />
            <Show when={props.unread > 0}>
                <Pill tone="accent">{badgeText(props.unread)}</Pill>
            </Show>
            <Show when={props.needsYou}>
                <span class="messages-rail__needs" role="img" aria-label="Needs you" />
            </Show>
        </span>
    );
}

/**
 * Put the Messages button in the pane's header while the pane is narrow and the panel is on, and take it away when
 * either stops. Its title says what the button holds.
 */
export function publishHeaderButton(
    blockId: string,
    show: Accessor<boolean>,
    unread: Accessor<number>,
    needsYou: Accessor<boolean>,
    toggle: () => void
): void {
    createEffect(() => {
        if (!show()) return publishMessagesHeaderButton(blockId, null);
        const n = unread();
        const needs = needsYou();
        const title = `Messages${n ? `, ${n} unread` : ""}${needs ? ", needs you" : ""}`;
        publishMessagesHeaderButton(blockId, {
            elemtype: "iconbutton",
            // A function, not a node: the pane's header is drawn in more than one place (the tile preview among
            // them), and each draws its own face. One node can only sit in one of them.
            icon: (() => <ButtonFace unread={n} needsYou={needs} />) as unknown as JSX.Element,
            title,
            fitContent: true,
            click: toggle,
        });
    });
    onCleanup(() => publishMessagesHeaderButton(blockId, null));
}
