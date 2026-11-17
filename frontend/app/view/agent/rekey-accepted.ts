// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Promotes an accepted pending message's row to the id history gives its record. A sent message's row shows at once
 * under a client id; when its transcript record is placed, the row takes the record's id, so a reload finds the same
 * row (collapsed state, a scroll anchor).
 *
 * The row can still be waiting in the pane's flush queue when its record is placed, and how long it waits depends on the
 * pane's flush rate (a hidden pane's flush is due 250 ms or more after its request), so the retry follows the queue,
 * not a count of frames: it goes on, a frame at a time, while the row is queued, up to REKEY_PENDING_MAX_MS.
 */

/** The longest a row's rekey waits for the row to leave the flush queue. */
export const REKEY_PENDING_MAX_MS = 5_000;

export interface RekeyDeps {
    /** The id history gives the record at `at`, or undefined if it holds no user message. */
    recordId: (record: string, at: { gen: string; line: number }) => string | undefined;
    /** Renames the row; true when the document held it. */
    rekey: (from: string, to: string) => boolean;
    /** True while the row waits in the pane's flush queue. */
    isQueued: (id: string) => boolean;
    onRekeyed: (to: string) => void;
    nextFrame: (run: () => void) => void;
    now: () => number;
}

export function createRekeyAccepted(deps: RekeyDeps) {
    return function rekeyAccepted(id: string, record: string, at: { gen: string; line: number }, since = deps.now()): void {
        const to = deps.recordId(record, at);
        if (!to || to === id) return;
        if (deps.rekey(id, to)) return deps.onRekeyed(to);
        if (deps.now() - since < REKEY_PENDING_MAX_MS && deps.isQueued(id)) {
            deps.nextFrame(() => rekeyAccepted(id, record, at, since));
        }
    };
}
