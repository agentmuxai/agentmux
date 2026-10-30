// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Outline icons for the browser pane's toolbar buttons that have no outline in
 * the bundled Font Awesome: Home (its regular style has no house) and, for the
 * same stroke weight beside it, Bookmarks. Back, Forward, Reload and Stop are
 * line glyphs already; a solid house and bookmark among them read as a
 * different set. The size follows the button's font size and the colour its
 * text colour.
 */

import type { JSX } from "solid-js";

function OutlineIcon(props: { class?: string; children: JSX.Element }): JSX.Element {
    return (
        <svg
            class={props.class}
            width="1em"
            height="1em"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            {props.children}
        </svg>
    );
}

export function ToolbarHomeIcon(props: { class?: string }): JSX.Element {
    return (
        <OutlineIcon class={props.class}>
            <path d="M2 7.6 8 2.4l6 5.2" />
            <path d="M3.6 6.6v7h8.8v-7" />
            <path d="M6.6 13.6V9.6h2.8v4" />
        </OutlineIcon>
    );
}

export function ToolbarBookmarkIcon(props: { class?: string }): JSX.Element {
    return (
        <OutlineIcon class={props.class}>
            <path d="M4 2.4h8v11.2L8 10.6l-4 3z" />
        </OutlineIcon>
    );
}
