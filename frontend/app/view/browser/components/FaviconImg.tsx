// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, Show, type JSX } from "solid-js";

export interface FaviconImgProps {
    src: string;
    size?: number;
}

// Renders a page favicon as an <img>. On load error (broken URL,
// blocked origin), swaps to a globe font-icon fallback so the pane
// header doesn't render a broken-image glyph.
export const FaviconImg = (props: FaviconImgProps): JSX.Element => {
    const [errored, setErrored] = createSignal(false);
    // Reset errored state whenever src changes — SolidJS re-uses the
    // same component instance across prop changes, so without this the
    // fallback persists for every URL after the first failure.
    createEffect(() => {
        props.src;
        setErrored(false);
    });
    const size = () => props.size ?? 16;
    return (
        <span
            class="browser-favicon"
            aria-hidden="true"
            style={{ display: "inline-flex", "align-items": "center", "line-height": 0 }}
        >
            <Show
                when={!errored() && props.src}
                fallback={<i class="fa-sharp fa-solid fa-globe" style={{ "font-size": `${size()}px` }} />}
            >
                <img
                    src={props.src}
                    width={size()}
                    height={size()}
                    alt=""
                    aria-hidden="true"
                    onError={() => setErrored(true)}
                />
            </Show>
        </span>
    );
};

FaviconImg.displayName = "FaviconImg";
