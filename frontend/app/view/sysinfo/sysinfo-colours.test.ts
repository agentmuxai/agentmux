// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The CPU chart's colour follows the theme: it is the theme's primary colour
 * (`--accent-color`), which every named theme overrides, not a fixed green.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { DefaultPlotMeta } from "./sysinfo-types";

const appDir = join(__dirname, "..", "..");
const themeScss = readFileSync(join(appDir, "theme.scss"), "utf8");

describe("sysinfo CPU colour is theme aware", () => {
    it("the CPU series uses the --sysinfo-cpu-color variable", () => {
        expect(DefaultPlotMeta.cpu.color).toBe("var(--sysinfo-cpu-color)");
    });

    it("--sysinfo-cpu-color is the theme's primary colour, not a hardcoded value", () => {
        const m = /--sysinfo-cpu-color:\s*([^;]+);/.exec(themeScss);
        expect(m, "no --sysinfo-cpu-color declaration in theme.scss").not.toBeNull();
        expect(m![1].trim()).toBe("var(--accent-color)");
    });

    it("every named theme sets its own --accent-color, so the CPU colour differs per theme", () => {
        const themesDir = join(appDir, "themes");
        const names = readdirSync(themesDir).filter((f) => f.endsWith(".scss") && f !== "index.scss");
        expect(names.length).toBeGreaterThan(5);
        for (const f of names) {
            expect(readFileSync(join(themesDir, f), "utf8"), f).toMatch(/--accent-color:/);
        }
    });
});

describe("sysinfo CPU and Mem are told apart", () => {
    const val = (css: string, name: string) => new RegExp(`${name}:\s*([^;]+);`).exec(css)?.[1].trim();

    it("Mem is green by default, CPU is the theme's primary colour", () => {
        expect(val(themeScss, "--sysinfo-mem-color")).toBe("#58c142");
        expect(val(themeScss, "--sysinfo-cpu-color")).toBe("var(--accent-color)");
    });

    // Monokai's primary colour is itself a lime green (#a6e22e), so a green Mem would
    // sit next to a green CPU there; it gets a cyan from its own palette instead.
    it("a theme whose primary colour is green gives Mem a different colour", () => {
        const themesDir = join(appDir, "themes");
        for (const f of readdirSync(themesDir).filter((n) => n.endsWith(".scss") && n !== "index.scss")) {
            const css = readFileSync(join(themesDir, f), "utf8");
            const accent = val(css, "--accent-color")?.toLowerCase() ?? "";
            const isGreen = /^#([a-f0-9]{2})([a-f0-9]{2})([a-f0-9]{2})$/.test(accent) && greenDominant(accent);
            if (isGreen) expect(val(css, "--sysinfo-mem-color"), `${f} needs its own Mem colour`).toBeDefined();
        }
    });
});

function greenDominant(hex: string): boolean {
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    return g > r + 20 && g > b + 40;
}
