// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §4.1, §5.1: a window's theme
// and opacity are its own when it has them, else the channel default from
// Settings → Appearance, which is exactly what every window showed before.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

import { opacityFor, themeFor } from "./window-appearance";

describe("themeFor", () => {
    it("is the window's own theme, else the default setting, else the built-in default", () => {
        expect(themeFor({ "window:theme": "dracula" }, { "window:theme": "light" })).toBe("dracula");
        expect(themeFor({}, { "window:theme": "light" })).toBe("light");
        expect(themeFor(undefined, undefined)).toBe("default");
        expect(themeFor({ "window:theme": "" }, {})).toBe("default");
    });
});

describe("opacityFor", () => {
    it("is the window's own opacity whatever the channel says, including fully opaque", () => {
        expect(opacityFor({ "window:opacity": 0.6 }, {})).toBe(0.6);
        expect(opacityFor({ "window:opacity": 1 }, { "window:transparent": true, "window:opacity": 0.5 })).toBe(1);
    });

    it("without one, is what every window showed before: the setting when see-through, else opaque", () => {
        expect(opacityFor({}, {})).toBe(1);
        expect(opacityFor({}, { "window:opacity": 0.5 })).toBe(1);
        expect(opacityFor({}, { "window:transparent": true, "window:opacity": 0.5 })).toBe(0.5);
        expect(opacityFor({}, { "window:blur": true, "window:opacity": 0.6 })).toBe(0.6);
        expect(opacityFor({}, { "window:transparent": true })).toBe(0.8);
    });
});

// The setting is only the default: code that reads it directly would show the
// channel's theme in a window that has its own. Only the resolver and the
// Settings pane (which edits the default) may name the key.
describe("window:theme", () => {
    const FRONTEND = join(__dirname, "..", "..");
    const ALLOWED = [
        join("app", "store", "window-appearance.ts"),
        join("app", "view", "settings", "sections", "appearance-section.tsx"),
    ];

    function sources(dir: string, out: string[] = []): string[] {
        for (const entry of readdirSync(dir, { withFileTypes: true })) {
            if (entry.name === "node_modules" || entry.name === "types" || entry.name.startsWith(".")) continue;
            const p = join(dir, entry.name);
            if (entry.isDirectory()) sources(p, out);
            else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) out.push(p);
        }
        return out;
    }

    it("is read only through currentWindowTheme()", () => {
        const offenders: string[] = [];
        for (const file of sources(FRONTEND)) {
            const rel = relative(FRONTEND, file);
            if (ALLOWED.includes(rel)) continue;
            readFileSync(file, "utf8")
                .split("\n")
                .forEach((line, i) => {
                    const t = line.trim();
                    if (t.startsWith("//") || t.startsWith("*")) return;
                    if (line.includes('"window:theme"')) offenders.push(`${rel}:${i + 1}: ${t}`);
                });
        }
        expect(offenders).toEqual([]);
    });
});
