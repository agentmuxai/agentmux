// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState } from "./types";

describe("window-opacity reducer", () => {
    it("applies opacity in range", () => {
        const { state, events } = update(initialState(), {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 0.75,
            source: "user",
        });
        expect(state.opacities["main"]).toBe(0.75);
        expect(events).toEqual([
            { type: "window-opacity-applied", label: "main", opacity: 0.75 },
        ]);
    });

    it("clears entry when opacity >= 1.0", () => {
        const seed = { opacities: { main: 0.75 } };
        const { state, events } = update(seed, {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 1.0,
            source: "user",
        });
        expect(state.opacities["main"]).toBeUndefined();
        expect(events[0].type).toBe("window-opacity-cleared");
    });

    it("clamps below minimum to 0.35", () => {
        const { state } = update(initialState(), {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 0.1,
            source: "user",
        });
        expect(state.opacities["main"]).toBe(0.35);
    });

    it("clamps above 1.0 and clears entry", () => {
        const { state } = update(initialState(), {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 1.5,
            source: "user",
        });
        expect(state.opacities["main"]).toBeUndefined();
    });

    // Floating panes share the same slice keyed by label — a floater's
    // "floating-<uuid>" label coexists with main-window labels
    // (instance-panel-floating-panes.md §3.2).
    it("tracks a floating-pane label independently of windows", () => {
        const first = update(initialState(), {
            type: "SetWindowOpacity",
            label: "floating-abc",
            opacity: 0.55,
            source: "user",
        });
        const { state } = update(first.state, {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 0.75,
            source: "user",
        });
        expect(state.opacities["floating-abc"]).toBe(0.55);
        expect(state.opacities["main"]).toBe(0.75);
    });

    it("WindowClosed removes entry", () => {
        const seed = { opacities: { main: 0.75, research: 0.5 } };
        const { state, events } = update(seed, { type: "WindowClosed", label: "main" });
        expect(state.opacities["main"]).toBeUndefined();
        expect(state.opacities["research"]).toBe(0.5);
        expect(events[0].type).toBe("window-opacity-entry-removed");
    });

    it("does not mutate the original state", () => {
        const original = initialState();
        update(original, {
            type: "SetWindowOpacity",
            label: "main",
            opacity: 0.75,
            source: "user",
        });
        expect(original.opacities["main"]).toBeUndefined();
    });
});
