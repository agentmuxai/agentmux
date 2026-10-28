// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const screenshot = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ browserPanes: { screenshot } }) }));

import { useDragSnapshot } from "./use-drag-snapshot";

/** A press on a pane header, which is what starts a prewarm. */
function press() {
    const header = document.createElement("div");
    header.setAttribute("draggable", "true");
    document.body.appendChild(header);
    header.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
    header.remove();
}

function mount(unavailable: () => boolean = () => false) {
    const diag = vi.fn();
    const model = { blockId: "b1", closed: false } as never;
    let dispose!: () => void;
    createRoot((d) => {
        dispose = d;
        useDragSnapshot({
            model,
            placeholderRef: () => document.body as HTMLDivElement,
            paneRect: () => ({ x: 0, y: 0, width: 800, height: 600 }),
            paneCreated: () => true,
            unavailable,
            diag,
        });
    });
    return { diag, dispose };
}

describe("useDragSnapshot — a capture that can't be answered must not hold the host's connections", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        screenshot.mockReset();
    });
    afterEach(() => vi.useRealTimers());

    it("gives up on a capture after two seconds and takes none from this pane for ten more", async () => {
        // A page that never paints: the request ends only when it is aborted.
        screenshot.mockImplementation(
            (_id: string, opts: { signal: AbortSignal }) =>
                new Promise((_, reject) => opts.signal.addEventListener("abort", () => reject(new Error("aborted")))),
        );
        const { diag, dispose } = mount();
        press();
        await vi.advanceTimersByTimeAsync(0);
        expect(screenshot).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(2000);
        expect(diag).toHaveBeenCalledWith(expect.stringContaining("capture failed"));
        // A press during the backoff takes nothing.
        press();
        await vi.advanceTimersByTimeAsync(0);
        expect(screenshot).toHaveBeenCalledTimes(1);
        // After it, the pane tries again.
        await vi.advanceTimersByTimeAsync(10_000);
        press();
        await vi.advanceTimersByTimeAsync(0);
        expect(screenshot).toHaveBeenCalledTimes(2);
        dispose();
    });

    it("takes nothing from a page that is hidden, asleep or frozen", async () => {
        const { dispose } = mount(() => true);
        press();
        await vi.advanceTimersByTimeAsync(0);
        expect(screenshot).not.toHaveBeenCalled();
        dispose();
    });

    it("reuses a capture started a moment ago instead of taking another", async () => {
        screenshot.mockImplementation(() => new Promise(() => {}));
        const { dispose } = mount();
        press();
        press();
        press();
        await vi.advanceTimersByTimeAsync(0);
        expect(screenshot).toHaveBeenCalledTimes(1);
        dispose();
    });
});
