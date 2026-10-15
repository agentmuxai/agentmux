// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import * as z from "./zindex";

const theme = readFileSync(new URL("./theme.scss", import.meta.url), "utf8");
const indexHtml = readFileSync(new URL("../../index.html", import.meta.url), "utf8");

/** The number theme.scss declares for `--z-name`, or undefined. */
function declared(name: string): number | undefined {
    const value = theme.match(new RegExp(String.raw`^\s*--z-${name}:\s*([^;]+);`, "m"))?.[1].trim();
    return value === undefined ? undefined : Number(value);
}

const LAYERS: Record<string, number> = {
    "capture-indicator": z.Z_CAPTURE_INDICATOR,
    transient: z.Z_TRANSIENT,
    "js-menu": z.Z_JS_MENU,
    splash: z.Z_SPLASH,
    "dev-diagnostics": z.Z_DEV_DIAGNOSTICS,
    "dev-hud": z.Z_DEV_HUD,
};

describe("z-index layers match theme.scss", () => {
    it.each(Object.entries(LAYERS))("--z-%s is %i", (name, value) => {
        expect(declared(name)).toBe(value);
    });

    it("zindex.ts exports no layer the test lacks", () => {
        expect(Object.values(z).sort()).toEqual(Object.values(LAYERS).sort());
    });

    it("the layers keep their order around the window's top layers", () => {
        expect(declared("flyout-menu")).toBeLessThan(z.Z_CAPTURE_INDICATOR);
        expect(z.Z_CAPTURE_INDICATOR).toBeLessThan(z.Z_TRANSIENT);
        expect(declared("top")).toBeGreaterThan(z.Z_TRANSIENT);
        expect(z.Z_JS_MENU).toBeGreaterThan(declared("top")!);
        expect(z.Z_DEV_DIAGNOSTICS).toBeGreaterThan(z.Z_SPLASH);
        expect(z.Z_DEV_HUD).toBeGreaterThan(z.Z_DEV_DIAGNOSTICS);
    });

    it("index.html's splash, set before any stylesheet, uses --z-splash's number", () => {
        const rule = indexHtml.match(/#startup-loading\s*\{([^}]*)\}/)?.[1] ?? "";
        expect(rule.match(/z-index:\s*(\d+)/)?.[1]).toBe(String(z.Z_SPLASH));
    });
});
