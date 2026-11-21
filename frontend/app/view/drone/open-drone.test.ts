// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ open: vi.fn(async () => {}) }));
vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: (...a: unknown[]) => (h.open as (...x: unknown[]) => Promise<void>)(...a),
}));

import { droneOpenRequest, openDroneGraph } from "./open-drone";

describe("open-drone", () => {
    it("asks for the Drone pane with the drone's id in its meta, both for a new pane and for one already open", async () => {
        await openDroneGraph("d-7");
        expect(h.open).toHaveBeenCalledWith("drone", { meta: { view: "drone", "drone:open": "d-7" } }, { "drone:open": "d-7" });
    });

    it("reads a request only when the meta holds a drone id", () => {
        expect(droneOpenRequest({ "drone:open": "d-7" })).toBe("d-7");
        for (const meta of [undefined, null, {}, { "drone:open": null }, { "drone:open": "" }, { "drone:open": 7 }]) {
            expect(droneOpenRequest(meta as Record<string, unknown> | null | undefined)).toBeNull();
        }
    });
});
