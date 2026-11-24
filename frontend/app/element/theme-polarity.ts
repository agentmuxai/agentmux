// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the window's theme is a light one, as a reactive value read from the `data-theme-polarity` marker on <html>
 * (app.tsx sets it for every light theme and removes it for the rest). For a library that paints its own colours at
 * render time (a diagram) and so has to be told the polarity and re-run when the theme flips; code that only colours
 * text uses the CSS rule on the same marker (shiki-themed.scss) and needs nothing from here.
 */

import { createSignal, type Accessor } from "solid-js";

export type ThemePolarity = "light" | "dark";

const read = (): ThemePolarity =>
    globalThis.document?.documentElement?.getAttribute("data-theme-polarity") === "light" ? "light" : "dark";

const [polarity, setPolarity] = createSignal<ThemePolarity>(read());
let watching = false;

/** The current polarity; the first call starts watching the marker. */
export function themePolarity(): Accessor<ThemePolarity> {
    if (!watching && globalThis.document?.documentElement && typeof MutationObserver !== "undefined") {
        watching = true;
        setPolarity(read());
        new MutationObserver(() => setPolarity(read())).observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-theme-polarity"],
        });
    }
    return polarity;
}
