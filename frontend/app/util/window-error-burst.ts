// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Counts the throws a window could not contain, so a window that keeps throwing can offer a reload. One throw is
 * a bug; a burst of them is a window whose bindings Solid has probably abandoned (see workspace/pane-watchdog.ts).
 * Fed by the page's uncaught-error handler (log/error-forwarder.ts) and by the root handler of the app
 * (containBindingErrors). A rejected promise is not counted: a failed request makes one, and a window offline is not damaged.
 */
export const BURST_COUNT = 5;
export const BURST_WINDOW_MS = 30_000;

export interface ErrorBurst {
    /** Notes one error. True when this one makes `count` within `windowMs`; the count starts again after that. */
    note: () => boolean;
}

export function createErrorBurst(opts: { count?: number; windowMs?: number; now?: () => number } = {}): ErrorBurst {
    const count = opts.count ?? BURST_COUNT;
    const windowMs = opts.windowMs ?? BURST_WINDOW_MS;
    const now = opts.now ?? Date.now;
    let stamps: number[] = [];
    return {
        note() {
            const t = now();
            stamps = stamps.filter((s) => t - s < windowMs);
            stamps.push(t);
            if (stamps.length < count) return false;
            stamps = [];
            return true;
        },
    };
}

const windowBurst = createErrorBurst();
const listeners = new Set<() => void>();

/** One error the window did not contain. Never throws. */
export function noteWindowError(): void {
    try {
        if (!windowBurst.note()) return;
        for (const fn of [...listeners]) fn();
    } catch {
        // Counting must never add a fault to the one it counts.
    }
}

/** `fn` runs each time the window's errors reach a burst. Returns the way to stop. */
export function onWindowErrorBurst(fn: () => void): () => void {
    listeners.add(fn);
    return () => listeners.delete(fn);
}
