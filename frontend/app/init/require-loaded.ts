// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A startup failure the app cannot render past — a core object (client,
 * window, tab, workspace, layout) that did not load.
 *
 * Exists so `initMuxWrap`'s catch can tell these apart from the late,
 * tolerable failures it deliberately swallows: only a fatal one is re-thrown,
 * so the `showStartupError` card in `initHostMux` actually gets a chance to
 * render instead of the window revealing itself blank.
 */
export class MuxInitFatalError extends Error {
    constructor(message: string) {
        super(message);
        this.name = "MuxInitFatalError";
    }
}

/**
 * Assert that a core startup object actually loaded.
 *
 * These loads can legitimately come back `null` — a tab id that no longer
 * exists, or a backend that answered without a row. Reading a field off that
 * null threw `TypeError: Cannot read properties of null (reading
 * 'layoutstate')` from inside init, which swallowed it, so the window revealed
 * itself completely blank with nothing but a console line to go on.
 *
 * Naming the object and its id makes the failure diagnosable; the
 * `MuxInitFatalError` type is what gets it in front of the user at all.
 */
export function requireLoaded<T>(value: T | null | undefined, kind: string, id: string): T {
    if (value == null) {
        throw new MuxInitFatalError(
            `startup: ${kind} "${id}" did not load — cannot initialize the window without it`
        );
    }
    return value;
}
