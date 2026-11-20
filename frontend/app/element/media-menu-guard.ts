// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One place that keeps the engine's own right-click menu from opening over a video or audio
 * element, on every surface that shows one (the Editor's media viewer, an agent's inline media,
 * the lightbox, a Files preview).
 *
 * The page asks for its own menu by cancelling the `contextmenu` event. A media element's native
 * controls are a separate tree inside the element, and a right-click on them gave the engine's menu
 * (a modal native menu on macOS that blocks the browser process until it is dismissed) whenever the
 * surface had not cancelled the event itself: the Editor's viewer relies on the pane body to do it
 * further up. The guard cancels the event for any media target at the top of the page, before it
 * reaches anyone. That only stops the engine's menu: the event still goes on to the pane body,
 * the lightbox or whoever builds the page's own menu, so the menu a surface shows is unchanged.
 */

/** Whether a right-click event landed on a video or audio element, its native controls included. */
export function isMediaTarget(e: Event): boolean {
    if (e.target instanceof HTMLMediaElement) return true;
    return e.composedPath().some((node) => node instanceof HTMLMediaElement);
}

/** Install the guard on a document; returns the function that removes it. */
export function installMediaMenuGuard(doc: Document = document): () => void {
    const guard = (e: Event): void => {
        if (isMediaTarget(e)) e.preventDefault();
    };
    // Capture: the first listener to run, so no handler further down can leave the event alone.
    doc.addEventListener("contextmenu", guard, true);
    return () => doc.removeEventListener("contextmenu", guard, true);
}
