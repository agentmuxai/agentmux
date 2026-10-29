// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sends native browser panes their rects: one request per window, carrying
 * every pane that changed.
 *
 * Each pane used to send its own `browser_pane_resize` on every frame its
 * placeholder moved. In a window drag over a tab of nine panes that was ~190
 * requests a second, up to 111 in flight, each applied one at a time by the
 * host, so a pane was drawn where a request from half a second ago put it
 * (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md).
 *
 * Now a pane only records its newest rect. The window's shared observer
 * (`pane-resize-observer.ts`) sends them from the rendering step that laid the
 * panes out (`flushNow`); every other source sends once the current task is
 * done, so every pane's callback for this frame has run. The host applies a
 * batch in one step and coalesces what is waiting for its UI thread, so a
 * second batch may go out once the first has been out for a frame, without
 * waiting for the page to process the host's reply: on a busy page that reply
 * waits behind everything else the page is doing, and the panes fall behind
 * the chrome around them.
 * The batches carry an order (`RectsOrder`), so the host drops a pane's rect
 * that a newer batch has already replaced.
 */

export type PaneRectUpdate = { blockId: string } & HostRect;

/** Which batch this is, so the host can tell an older one that arrives late. */
export interface RectsOrder {
    /** This page's start, so a reloaded page's batches are always newer. */
    epoch: number;
    /** Counts up per window within the page. */
    seq: number;
}

/** Sends one batch; resolves when the host has applied it. */
export type SendRects = (updates: PaneRectUpdate[]) => Promise<unknown>;

export interface PaneRectBatcher {
    /** Record a pane's newest rect; it goes in the next batch. */
    set(blockId: string, rect: HostRect): void;
    /** Drop a pane's unsent rect, e.g. when its view unmounts. */
    forget(blockId: string): void;
    /** Send what is recorded now, from the caller's own task. */
    flushNow(): void;
}

/** The time the batcher waits on, replaceable in tests. */
export interface BatcherClock {
    now(): number;
    after(ms: number, cb: () => void): void;
}

const realClock: BatcherClock = {
    now: () => performance.now(),
    after: (ms, cb) => void setTimeout(cb, ms),
};

/** How long a batch is out before the next one may go without its reply (a 60 Hz frame). */
const FRAME_MS = 16;
/** Batches the page has out at once; the host coalesces them, but a stuck host must not be buried. */
const MAX_IN_FLIGHT = 3;

/** Runs `cb` as its own task, after the current one (no 4 ms timer clamp). */
function nextTask(cb: () => void): void {
    const ch = new MessageChannel();
    ch.port1.onmessage = () => {
        ch.port1.close();
        cb();
    };
    ch.port2.postMessage(null);
}

export function createPaneRectBatcher(
    send: SendRects,
    schedule: (cb: () => void) => void = nextTask,
    clock: BatcherClock = realClock,
): PaneRectBatcher {
    const pending = new Map<string, HostRect>();
    let scheduled = false;
    let inFlight = 0;
    let lastSentAt = -Infinity;
    let retryScheduled = false;

    const flush = () => {
        scheduled = false;
        if (pending.size === 0 || inFlight >= MAX_IN_FLIGHT) return;
        if (inFlight > 0) {
            // A batch is out: the next goes once it has been out for a frame, not on its reply.
            const wait = FRAME_MS - (clock.now() - lastSentAt);
            if (wait > 0) {
                if (!retryScheduled) {
                    retryScheduled = true;
                    clock.after(wait, () => {
                        retryScheduled = false;
                        flush();
                    });
                }
                return;
            }
        }
        const batch: PaneRectUpdate[] = [];
        for (const [blockId, rect] of pending) batch.push({ blockId, ...rect });
        pending.clear();
        inFlight++;
        lastSentAt = clock.now();
        send(batch)
            .catch(() => {})
            .finally(() => {
                inFlight--;
                // Newer rects arrived while this batch was out: send them now
                // rather than waiting for something else to schedule a flush.
                if (pending.size > 0) flush();
            });
    };

    return {
        set(blockId, rect) {
            pending.set(blockId, { x: rect.x, y: rect.y, width: rect.width, height: rect.height });
            if (scheduled) return;
            scheduled = true;
            schedule(flush);
        },
        forget(blockId) {
            pending.delete(blockId);
        },
        flushNow: flush,
    };
}

/** This page's start; a reloaded page's batches are newer than its predecessor's. */
const PAGE_EPOCH = Math.floor(performance.timeOrigin);

/**
 * The host's batch command, falling back to one `resize` per pane if this host
 * doesn't have it (an older build): the panes still move, just as before.
 */
export function hostSendRects(api: BrowserPaneHostApi, windowLabel: string): SendRects {
    let batchUnsupported = false;
    let seq = 0;
    const perPane = (updates: PaneRectUpdate[]) =>
        Promise.all(updates.map(({ blockId, ...rect }) => api.resize(blockId, rect).catch(() => {})));
    return async (updates) => {
        if (batchUnsupported) return perPane(updates);
        try {
            await api.setRects(windowLabel, updates, { epoch: PAGE_EPOCH, seq: ++seq });
        } catch (e) {
            if (!/unknown command/i.test(String(e))) throw e;
            batchUnsupported = true;
            return perPane(updates);
        }
    };
}

const batchers = new Map<string, PaneRectBatcher>();

/** The batcher for one window: every pane in it shares a single request. */
export function paneRectBatcher(windowLabel: string, api: () => BrowserPaneHostApi): PaneRectBatcher {
    let b = batchers.get(windowLabel);
    if (!b) {
        let send: SendRects | null = null;
        b = createPaneRectBatcher((updates) => (send ??= hostSendRects(api(), windowLabel))(updates));
        batchers.set(windowLabel, b);
    }
    return b;
}
