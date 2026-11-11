// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for shiki-no-themes.mjs. The last test runs it against the installed Shiki, so an update that
// changes how bundle-web.mjs imports its themes fails here, not as 65 theme chunks in the next build.
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isShikiWebBundle, shikiNoThemes, withoutBundledThemes } from "./shiki-no-themes.mjs";

describe("isShikiWebBundle", () => {
    it("matches Shiki's web bundle on either path style, with a query", () => {
        expect(isShikiWebBundle("C:\\repo\\node_modules\\shiki\\dist\\bundle-web.mjs")).toBe(true);
        expect(isShikiWebBundle("/repo/node_modules/shiki/dist/bundle-web.mjs?v=1")).toBe(true);
    });
    it("leaves the full bundle and other packages alone", () => {
        expect(isShikiWebBundle("/repo/node_modules/shiki/dist/bundle-full.mjs")).toBe(false);
        expect(isShikiWebBundle("/repo/node_modules/other/dist/bundle-web.mjs")).toBe(false);
    });
});

describe("withoutBundledThemes", () => {
    const SOURCE = [
        'import { bundledThemes, bundledThemesInfo } from "./themes.mjs";',
        "const x = { themes: bundledThemes };",
        "export { bundledThemes, bundledThemesInfo };",
    ].join(String.fromCharCode(10));


    it("swaps the theme import for empty tables the export list still names", () => {
        const out = withoutBundledThemes(SOURCE);
        expect(out).toContain("const bundledThemes = {};");
        expect(out).toContain("const bundledThemesInfo = [];");
        expect(out).not.toContain("themes.mjs");
        expect(out).toContain("themes: bundledThemes");
        expect(out).toContain("export { bundledThemes, bundledThemesInfo };");
    });
    it("throws when Shiki's file no longer looks like that", () => {
        expect(() => withoutBundledThemes("export const a = 1;")).toThrow(/shiki-no-themes/);
    });
});

describe("shikiNoThemes", () => {
    it("rewrites only the web bundle", () => {
        const plugin = shikiNoThemes();
        expect(plugin.transform("export const a = 1;", "/repo/src/a.ts")).toBeNull();
    });
    it("handles the Shiki that is installed", () => {
        const here = path.dirname(fileURLToPath(import.meta.url));
        const file = path.resolve(here, "../node_modules/shiki/dist/bundle-web.mjs");
        const out = shikiNoThemes().transform(fs.readFileSync(file, "utf8"), file);
        expect(out.code).toContain("const bundledThemes = {};");
        expect(out.code).toContain("const bundledThemesInfo = [];");
        expect(out.code).not.toContain("./themes.mjs");
    });
});
