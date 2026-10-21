// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The part of rxjs's `Subject` the app used (`store/mps.ts` file subjects):
 * `next`, `subscribe` returning something to `unsubscribe`, and `complete`.
 * As with rxjs, a listener that throws doesn't stop the others; its error is
 * rethrown on its own task so it still reaches the console and error reporting.
 * SPEC_MARKDOWN_CODE_ON_SHIKI_AND_REDUNDANT_LIBRARIES_2026_10_21.md §6.2.
 */

export interface Subscription {
    unsubscribe(): void;
}

export class Emitter<T> {
    // One entry per subscribe call, so the same function subscribed twice is
    // called twice and unsubscribed once each.
    private readonly listeners = new Set<{ fn: (value: T) => void }>();
    private closed = false;

    next(value: T): void {
        if (this.closed) return;
        for (const entry of [...this.listeners]) {
            if (!this.listeners.has(entry)) continue;
            try {
                entry.fn(value);
            } catch (e) {
                setTimeout(() => {
                    throw e;
                });
            }
        }
    }

    subscribe(fn: (value: T) => void): Subscription {
        if (this.closed) return { unsubscribe: () => {} };
        const entry = { fn };
        this.listeners.add(entry);
        return { unsubscribe: () => void this.listeners.delete(entry) };
    }

    /** No more values; every listener is dropped. */
    complete(): void {
        this.closed = true;
        this.listeners.clear();
    }

    get observed(): boolean {
        return this.listeners.size > 0;
    }
}
