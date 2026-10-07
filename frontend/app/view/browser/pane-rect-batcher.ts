// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sends native browser panes their rects: one request per window, carrying
 * every pane that changed, never more than one in flight.
 *
 * Each pane used to send its own `browser_pane_resize` on every frame its
 * placeholder moved. In a window drag over a tab of nine panes that was ~190
 * requests a second, up to 111 in flight, each applied one at a time by the
 * host, so a pane was drawn where a request from half a second ago put it
 * (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md).
 *
 * Now a pane only records its newest rect. Once the current task is done
 * (so every pane's ResizeObserver callback for this frame has run) the
 * batcher sends everything recorded in one `browser_panes_set_rects`. While
 * that is in flight, newer rects replace older ones, and the moment it
 * returns whatever is newest goes out. The host applies a batch in one step
 * and answers once the panes have moved, so a drag never builds a queue.
 */

export type PaneRectUpdate = { blockId: string } & HostRect;

/** Sends one batch; resolves when the host has applied it. */
export type SendRects = (updates: PaneRectUpdate[]) => Promise<unknown>;

export interface PaneRectBatcher {
    /** Record a pane's newest rect; it goes in the next batch. */
    set(blockId: string, rect: HostRect): void;
    /** Drop a pane's unsent rect, e.g. when its view unmounts. */
    forget(blockId: string): void;
}

/** Runs `cb` as its own task, after the current one (no 4 ms timer clamp). */
export function nextTask(cb: () => void): void {
    const ch = new MessageChannel();
    ch.port1.onmessage = () => {
        ch.port1.close();
        cb();
    };
    ch.port2.postMessage(null);
}

export function createPaneRectBatcher(send: SendRects, schedule: (cb: () => void) => void = nextTask): PaneRectBatcher {
    const pending = new Map<string, HostRect>();
    let scheduled = false;
    let inFlight = false;

    const flush = () => {
        scheduled = false;
        if (inFlight || pending.size === 0) return;
        const batch: PaneRectUpdate[] = [];
        for (const [blockId, rect] of pending) batch.push({ blockId, ...rect });
        pending.clear();
        inFlight = true;
        send(batch)
            .catch(() => {})
            .finally(() => {
                inFlight = false;
                // Newer rects arrived while this batch was out: send them now
                // rather than waiting for something else to schedule a flush.
                if (pending.size > 0) flush();
            });
    };

    return {
        set(blockId, rect) {
            pending.set(blockId, { x: rect.x, y: rect.y, width: rect.width, height: rect.height });
            if (scheduled || inFlight) return;
            scheduled = true;
            schedule(flush);
        },
        forget(blockId) {
            pending.delete(blockId);
        },
    };
}

/**
 * The host's batch command, falling back to one `resize` per pane if this host
 * doesn't have it (an older build): the panes still move, just as before.
 */
export function hostSendRects(api: BrowserPaneHostApi, windowLabel: string): SendRects {
    let batchUnsupported = false;
    const perPane = (updates: PaneRectUpdate[]) =>
        Promise.all(updates.map(({ blockId, ...rect }) => api.resize(blockId, rect).catch(() => {})));
    return async (updates) => {
        if (batchUnsupported) return perPane(updates);
        try {
            await api.setRects(windowLabel, updates);
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
