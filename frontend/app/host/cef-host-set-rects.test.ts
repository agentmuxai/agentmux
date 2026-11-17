// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeFastCommand = vi.fn(async () => true);
vi.mock("@/app/platform/ipc", () => ({ invokeBrowserApi: vi.fn(), invokeCommand: vi.fn() }));
vi.mock("@/app/platform/ipc-query", () => ({ invokeFastCommand }));
vi.mock("@/app/view/browser/leave-prompt", () => ({ takePersonClose: vi.fn() }));

const { cefBrowserPanes } = await import("./cef-host-commands");

const rect = (x: number) => ({ x, y: 0, width: 100, height: 50 });

describe("cefBrowserPanes.setRects", () => {
    beforeEach(() => {
        invokeFastCommand.mockClear();
        Object.defineProperty(window, "innerWidth", { configurable: true, value: 2000 });
        Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });
    });

    it("sends each rect with the base it was measured against, whatever the window is now", async () => {
        const measured: PaneLayoutBase = { viewport: { width: 1590, height: 800 }, layout: { x: 0, y: 70, width: 1590, height: 680 } };
        await cefBrowserPanes.setRects("main", [{ blockId: "a", ...rect(1), base: measured }]);
        const [cmd, args] = invokeFastCommand.mock.calls[0] as unknown as [string, Record<string, unknown>];
        expect(cmd).toBe("browser_panes_set_rects");
        expect(args.rects).toEqual([{ block_id: "a", ...rect(1), ...measured }]);
        // Every rect has its own: the batch carries none that says the window is 2000 wide.
        expect(args.viewport).toBeUndefined();
    });

    it("reads the batch's base now only for a rect that has none", async () => {
        const measured: PaneLayoutBase = { viewport: { width: 1590, height: 800 } };
        await cefBrowserPanes.setRects("main", [
            { blockId: "a", ...rect(1), base: measured },
            { blockId: "b", ...rect(2) },
        ]);
        const args = (invokeFastCommand.mock.calls[0] as unknown as [string, Record<string, unknown>])[1];
        expect(args.rects).toEqual([
            { block_id: "a", ...rect(1), ...measured },
            { block_id: "b", ...rect(2) },
        ]);
        expect(args.viewport).toEqual({ width: 2000, height: 800 });
    });
});
