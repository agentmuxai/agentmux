// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState } from "./types";

describe("window-opacity reducer", () => {
    it("applies opacity in range", () => {
        const { state, events } = update(initialState(), {
            type: "SetWindowOpacity",
            windowId: "win-1",
            label: "main",
            opacity: 0.75,
            source: "user",
        });
        expect(state.opacities["win-1"]).toBe(0.75);
        expect(events).toEqual([
            { type: "window-opacity-applied", windowId: "win-1", label: "main", opacity: 0.75 },
        ]);
    });

    it("clears entry when opacity >= 1.0", () => {
        const seed = { opacities: { "win-1": 0.75 } };
        const { state, events } = update(seed, {
            type: "SetWindowOpacity",
            windowId: "win-1",
            label: "main",
            opacity: 1.0,
            source: "user",
        });
        expect(state.opacities["win-1"]).toBeUndefined();
        expect(events[0].type).toBe("window-opacity-cleared");
    });

    it("clamps below minimum to 0.35", () => {
        const { state } = update(initialState(), {
            type: "SetWindowOpacity",
            windowId: "win-1",
            label: "main",
            opacity: 0.1,
            source: "user",
        });
        expect(state.opacities["win-1"]).toBe(0.35);
    });

    it("clamps above 1.0 and clears entry", () => {
        const { state } = update(initialState(), {
            type: "SetWindowOpacity",
            windowId: "win-1",
            label: "main",
            opacity: 1.5,
            source: "user",
        });
        expect(state.opacities["win-1"]).toBeUndefined();
    });

    it("WindowClosed removes entry", () => {
        const seed = { opacities: { "win-1": 0.75, "win-2": 0.5 } };
        const { state, events } = update(seed, { type: "WindowClosed", windowId: "win-1" });
        expect(state.opacities["win-1"]).toBeUndefined();
        expect(state.opacities["win-2"]).toBe(0.5);
        expect(events[0].type).toBe("window-opacity-entry-removed");
    });

    it("does not mutate the original state", () => {
        const original = initialState();
        update(original, {
            type: "SetWindowOpacity",
            windowId: "win-1",
            label: "main",
            opacity: 0.75,
            source: "user",
        });
        expect(original.opacities["win-1"]).toBeUndefined();
    });
});
