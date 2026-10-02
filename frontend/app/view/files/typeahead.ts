// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type-ahead: typing moves focus to the next name starting with what was
 * typed. Keys typed within 500 ms build one prefix; after a pause the next key
 * starts a new one. Repeating one letter cycles through names starting with
 * it, as file managers do. docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.3.
 */

export const TYPEAHEAD_RESET_MS = 500;

export class TypeAhead {
    private buffer = "";
    private last = 0;

    /** The name to focus after typing `key`, or null for no match. */
    next(key: string, order: readonly string[], focus: string | null, now: number = Date.now()): string | null {
        if (now - this.last > TYPEAHEAD_RESET_MS) this.buffer = "";
        this.last = now;
        this.buffer += key.toLowerCase();
        const cycling = this.buffer.length > 1 && [...this.buffer].every((c) => c === this.buffer[0]);
        const prefix = cycling ? this.buffer[0] : this.buffer;
        const start = focus != null ? order.indexOf(focus) : -1;
        // A fresh prefix (or a cycling letter) starts after the focused row; a
        // growing prefix may stay on it.
        const from = this.buffer.length === 1 || cycling ? start + 1 : Math.max(start, 0);
        for (let k = 0; k < order.length; k++) {
            const name = order[(from + k) % order.length];
            if (name.toLowerCase().startsWith(prefix)) return name;
        }
        return null;
    }

    /** Whether a key typed now continues a prefix (so Space adds to it rather
     *  than toggling the selection). */
    active(now: number = Date.now()): boolean {
        return this.buffer !== "" && now - this.last <= TYPEAHEAD_RESET_MS;
    }

    reset(): void {
        this.buffer = "";
        this.last = 0;
    }
}
