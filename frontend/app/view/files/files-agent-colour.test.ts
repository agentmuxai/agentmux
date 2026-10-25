// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's name in the Files sidebar is in its colour, and the colour reads on every theme's surface.
 * The raw colour of a yellow agent was 1.7:1 on the light themes (found by looking at the running app on
 * each theme); the name now takes the theme's text variant of its hue.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

const light = vi.hoisted(() => ({ on: false }));
vi.mock("@/app/block/pane-identity", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/block/pane-identity")>()),
    isLightThemeActive: () => light.on,
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { contrastRatio } from "@/app/block/pane-color-scheme";
import { readableAgentColor } from "./files-model";

// The surfaces of a dark and a light theme (default, and light).
const DARK_SURFACE = "#222222";
const LIGHT_SURFACE = "#f6f6f7";
const HUES = ["#eab308", "#facc15", "#22c55e", "#3b82f6", "#f97316", "#ec4899", "#a855f7"];

describe("readableAgentColor", () => {
    afterEach(() => {
        light.on = false;
    });

    it("reads at 4.5:1 or better on a light theme, for every hue", () => {
        light.on = true;
        for (const hue of HUES) {
            const c = readableAgentColor(hue);
            expect(c, hue).toMatch(/^#[0-9a-f]{6}$/);
            expect(contrastRatio(c, LIGHT_SURFACE), `${hue} -> ${c}`).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("reads at 4.5:1 or better on a dark theme, for every hue", () => {
        for (const hue of HUES) {
            const c = readableAgentColor(hue);
            expect(contrastRatio(c, DARK_SURFACE), `${hue} -> ${c}`).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("takes a different lightness on a light theme than on a dark one", () => {
        const dark = readableAgentColor("#eab308");
        light.on = true;
        expect(readableAgentColor("#eab308")).not.toBe(dark);
    });
});
