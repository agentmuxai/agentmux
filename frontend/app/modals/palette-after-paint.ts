// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The palette's work that is not needed to show it (the rows past the first ones, which labels are cut) runs after the
// frame that shows it. A requestAnimationFrame callback is not that: it runs in the next rendering step, before the
// frame is drawn, so what it builds is in the first paint and what it reads forces its layout there.

/**
 * Runs `cb` after the next frame is drawn: a requestAnimationFrame (before the draw), then a task queued from it (after
 * it). Returns what cancels it.
 */
export function afterNextPaint(cb: () => void): () => void {
    let live = true;
    let channel: MessageChannel | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const frame = requestAnimationFrame(() => {
        if (!live) return;
        if (typeof MessageChannel === "undefined") {
            timer = setTimeout(() => live && cb(), 0);
            return;
        }
        const ch = (channel = new MessageChannel());
        ch.port1.onmessage = () => {
            ch.port1.close();
            if (live) cb();
        };
        ch.port2.postMessage(0);
    });
    return () => {
        live = false;
        cancelAnimationFrame(frame);
        clearTimeout(timer);
        channel?.port1.close();
    };
}
