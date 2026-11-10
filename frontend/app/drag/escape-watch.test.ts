// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const os = vi.hoisted(() => ({ windows: true }));
const calls: string[] = [];
let escapeListener: (() => void) | null = null;
const api = {
    listen: vi.fn(async (_event: string, cb: () => void) => {
        escapeListener = cb;
        return () => {};
    }),
    getWindowLabel: vi.fn(async () => "main"),
    startTabDragTracking: vi.fn(async (args: unknown) => void calls.push(`start ${JSON.stringify(args)}`)),
    stopTabDragTracking: vi.fn(async () => void calls.push("stop")),
};
vi.mock("@/store/global", () => ({ getApi: () => api }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));
vi.mock("@/util/platformutil", () => ({ isWindows: () => os.windows }));
const followPaneDrag = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("./pane-live-follow", () => ({ followPaneDrag }));

import { beginDrag, endDrag } from "./drag-session";
import { watchEscapeDuringDrag } from "./escape-watch";
import { hostEscapeSeen, resetHostEscape } from "./host-escape";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
    os.windows = true;
    calls.length = 0;
    resetHostEscape();
    endDrag("cancel");
    vi.clearAllMocks();
});

describe("watchEscapeDuringDrag", () => {
    it("arms the host's Escape hook for the drag, and stops it when the session ends", async () => {
        beginDrag("tile", { nodeId: "n1" });
        watchEscapeDuringDrag();
        await settle();
        expect(calls).toEqual([`start ${JSON.stringify({ sourceWindowLabel: "main", escapeOnly: true })}`]);
        endDrag("dragend");
        await settle();
        expect(calls[1]).toBe("stop");
    });

    it("for a pane drag, asks the host to move the pane's picture too, once the hook is armed", async () => {
        beginDrag("tile", { nodeId: "n1" });
        watchEscapeDuringDrag({ blockId: "b1" });
        await settle();
        expect(calls).toEqual([`start ${JSON.stringify({ sourceWindowLabel: "main", escapeOnly: true, paneFollow: true })}`]);
        expect(followPaneDrag).toHaveBeenCalledWith("b1", "main", undefined);
    });

    it("keeps the host's Escape for the end of the drag, and forgets it at the next drag", async () => {
        beginDrag("pane-tab", { blockId: "b1" });
        watchEscapeDuringDrag();
        await settle();
        escapeListener?.();
        expect(hostEscapeSeen()).toBe(true);
        endDrag("dragend");
        expect(hostEscapeSeen()).toBe(true);
        beginDrag("tile", { nodeId: "n2" });
        expect(hostEscapeSeen()).toBe(false);
    });

    it("does nothing off Windows", async () => {
        os.windows = false;
        beginDrag("tile", { nodeId: "n1" });
        watchEscapeDuringDrag();
        await settle();
        expect(api.startTabDragTracking).not.toHaveBeenCalled();
    });
});
