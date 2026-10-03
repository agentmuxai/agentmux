// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { openTearOffWindow } from "./tear-off-pool-helper";

vi.mock("@/util/logger", () => ({ Logger: { warn: vi.fn(), info: vi.fn(), error: vi.fn(), debug: vi.fn() } }));

const tearOffPoolPromote = vi.fn();
const openWindowAtPosition = vi.fn();
const api = { tearOffPoolPromote, openWindowAtPosition } as unknown as Parameters<typeof openTearOffWindow>[0];

describe("openTearOffWindow", () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it("uses the warm pool when it can, and never the cold path", async () => {
        tearOffPoolPromote.mockResolvedValue("pool-win");
        const dest = await openTearOffWindow(api, "ws-1", 10, 20, 800, 600, 3, 4, "snap");
        expect(dest).toEqual({ label: "pool-win", pooled: true });
        expect(tearOffPoolPromote).toHaveBeenCalledWith("ws-1", 10, 20, 800, 600, 3, 4, "snap");
        expect(openWindowAtPosition).not.toHaveBeenCalled();
    });

    it("falls back to the cold path when the pool rejects", async () => {
        tearOffPoolPromote.mockRejectedValue(new Error("pool exhausted"));
        openWindowAtPosition.mockResolvedValue("cold-win");
        const dest = await openTearOffWindow(api, "ws-1", 10, 20, 800, 600, 3, 4, "snap");
        expect(dest).toEqual({ label: "cold-win", pooled: false });
        expect(openWindowAtPosition).toHaveBeenCalledWith(10, 20, "ws-1", 800, 600, 3, 4);
    });

    it("rejects with the cold path's error when both paths fail", async () => {
        tearOffPoolPromote.mockRejectedValue(new Error("pool exhausted"));
        openWindowAtPosition.mockRejectedValue(new Error("create failed"));
        await expect(openTearOffWindow(api, "ws-1", 10, 20)).rejects.toThrow("create failed");
    });
});
