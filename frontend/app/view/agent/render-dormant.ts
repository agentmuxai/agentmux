// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether an agent pane should treat itself as dormant for rendering: its tab
 * is hidden (`usePaneTabVisibility`) OR the whole window can't be seen
 * (minimized; or fully covered, where the platform reports it: Windows doesn't), which
 * Chromium reports as `document.hidden`.
 *
 * A minimized window runs no animation frames and no ResizeObserver callbacks,
 * so everything the pane trims on geometry (a held row released once it
 * scrolls off the top, a tool body that stops building output while hidden)
 * never ran there, yet the stream kept mounting turns at every turn end:
 * four streaming agents grew ~42 MB/min of JS heap and ~4,000 mounted preview
 * lines each, all trimmed the moment the window came back. Counting a hidden
 * window as dormant routes it through the paths a hidden tab already takes
 * (`agent-dormancy.tsx`, `useLiveFeedRollOff`'s hidden-pane grace).
 */

import { createSignal, type Accessor } from "solid-js";

const [windowHidden, setWindowHidden] = createSignal(typeof document !== "undefined" && document.hidden);
let listening = false;

function listen(): void {
    if (listening || typeof document === "undefined") return;
    listening = true;
    setWindowHidden(document.hidden);
    document.addEventListener("visibilitychange", () => setWindowHidden(document.hidden));
}

/** True while the page itself is hidden (window minimized, or covered where reported). */
export function useWindowHidden(): Accessor<boolean> {
    listen();
    return windowHidden;
}

/** The pane's dormancy for rendering: its tab hidden, or its window. */
export function useRenderDormant(paneHidden: Accessor<boolean>): Accessor<boolean> {
    const hiddenWindow = useWindowHidden();
    return () => paneHidden() || hiddenWindow();
}
