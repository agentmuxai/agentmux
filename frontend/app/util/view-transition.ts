// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runs `update` inside a view transition where the browser has them, else plainly.
 *
 * A transition that the next one replaces (two window tab switches in quick
 * succession) is skipped, and its `ready` and `finished` promises reject with an
 * AbortError "Transition was skipped. New ViewTransition started". That is the
 * swap doing its job, not a failure, and nobody awaits those promises, so left
 * alone each one became an `unhandledrejection` the host logged as an error.
 * The update callback still runs for a skipped transition; any other rejection
 * is still reported.
 */
export function runInViewTransition(update: () => void): void {
    if (typeof document.startViewTransition !== "function") {
        update();
        return;
    }
    const transition = document.startViewTransition(update);
    for (const settled of [transition.ready, transition.finished]) settled.catch(ignoreSkipped);
}

function ignoreSkipped(err: unknown): void {
    if (err instanceof DOMException && err.name === "AbortError") return;
    // Not a skipped transition: let it surface as the unhandled rejection it would have been.
    Promise.reject(err);
}
