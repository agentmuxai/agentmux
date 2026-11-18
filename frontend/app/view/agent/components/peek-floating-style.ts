// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The inline style of PeekOverlay's panel, from where peek-placement.ts put it (before the pane's zoom is applied). */

import type { JSX } from "solid-js";
import type { PeekHorizontal, PeekVertical } from "./peek-placement";

/**
 * A panel that only shows information (it fits: nothing to scroll or select) takes no pointer events, so it can
 * never hold the wheel or the pointer wherever it is drawn: the wheel goes to what is under it, the transcript. Only
 * a panel that was cut and scrolls (`enterable`) is entered, for its scroll bar and text.
 */
export function peekFloatingStyle(hz: PeekHorizontal, vt: PeekVertical, enterable: boolean): JSX.CSSProperties {
    return {
        position: "fixed",
        left: `${hz.left}px`,
        top: `${vt.top}px`,
        ...(hz.shift > 0 ? { transform: `translateX(${-hz.shift * 100}%)` } : {}),
        "max-width": `${hz.maxWidth}px`,
        "min-width": `${hz.minWidth}px`,
        "max-height": `${vt.maxHeight}px`,
        "pointer-events": enterable ? "auto" : "none",
    };
}
