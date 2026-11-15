// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Keeps one binding's error from taking the rest of the window with it. Solid
 * abandons every effect and binding queued in a flush that throws, and a throw
 * from a computation with no error handler above it is what makes a flush throw.
 * With a handler at the root, the error is reported there instead and the flush
 * goes on: a "Stale read from <Show>." in a window-focus handler (2026-11-14)
 * had left a window with no tab displayed at all. An ErrorBoundary nearer the
 * error still takes it first, and shows its own fallback.
 */

import { catchError } from "solid-js";

/** Run `build` (the app's root component) under a handler that reports what no boundary below caught. */
export function containBindingErrors<T>(build: () => T, report: (message: string) => void): T {
    return catchError(build, (error) => {
        const text = error instanceof Error ? `${error.message}${error.stack ? `\n${error.stack}` : ""}` : String(error);
        report(`[ui] a binding threw; the window goes on without it: ${text}`);
    });
}
