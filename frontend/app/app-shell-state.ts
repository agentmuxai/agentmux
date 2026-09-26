// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the app shell renders, given whether the client and window records are
 * present and whether they have ever both been present in this page.
 *
 * - `app` — both present: render the workspace.
 * - `invalid` — never loaded: a genuine startup failure, show the error.
 * - `unloading` — loaded once, now gone: the window's record was deleted out
 *   from under a live page (closing the main window deletes it before the host
 *   closes the window; an API `CloseWindow` does the same). Not an error —
 *   render the bare background until the window goes away.
 *
 * `client == null` alone cannot tell "never loaded" from "already unloaded";
 * the latch can. See SPEC_SHUTDOWN_INVALID_CONFIGURATION_FLASH_2026_09_22 §9.
 */
export type AppShellState = "app" | "unloading" | "invalid";

export function appShellState(clientPresent: boolean, windowPresent: boolean, loadedOnce: boolean): AppShellState {
    if (clientPresent && windowPresent) {
        return "app";
    }
    return loadedOnce ? "unloading" : "invalid";
}
