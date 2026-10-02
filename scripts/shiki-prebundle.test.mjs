// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for shiki-prebundle.mjs. The last test runs it against the real
// shiki-highlighter.ts, so a grammar added there is covered without anyone
// touching vite.config.ts, and a change to the import style that the pattern
// no longer matches fails here instead of silently bringing the reload back.

import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { shikiPrebundleDeps } from "./shiki-prebundle.mjs";

describe("shikiPrebundleDeps", () => {
    it("collects grammar, engine and wasm specifiers, sorted and unique", () => {
        const src = `
            ["rust", ["rs"], () => import("shiki/langs/rust.mjs")],
            ["go", [], () => import("shiki/langs/go.mjs")],
            ["go2", [], () => import('shiki/langs/go.mjs')],
            import { createOnigurumaEngine } from "shiki/engine/oniguruma";
            engine: () => createOnigurumaEngine(import("shiki/wasm")),
        `;
        expect(shikiPrebundleDeps(src)).toEqual([
            "shiki/engine/oniguruma",
            "shiki/langs/go.mjs",
            "shiki/langs/rust.mjs",
            "shiki/wasm",
        ]);
    });

    it("ignores Shiki entry points Vite already scans, and type-only imports", () => {
        const src = `
            import type { DynamicImportLanguageRegistration } from "shiki/core";
            import { bundledLanguages } from "shiki/bundle/web";
        `;
        expect(shikiPrebundleDeps(src)).toEqual([]);
    });

    it("returns nothing for source with no Shiki imports", () => {
        expect(shikiPrebundleDeps("export const x = 1;")).toEqual([]);
    });

    it("covers every grammar loader in the real shiki-highlighter.ts", () => {
        const file = path.resolve(
            path.dirname(fileURLToPath(import.meta.url)),
            "../frontend/app/view/agent/components/shiki-highlighter.ts",
        );
        const src = fs.readFileSync(file, "utf8");
        const deps = shikiPrebundleDeps(src);
        // Every lazy grammar import in the file must be listed.
        const loaders = [...src.matchAll(/import\(\s*["'](shiki\/langs\/[^"']+)["']\s*\)/g)].map((m) => m[1]);
        expect(loaders.length).toBeGreaterThanOrEqual(24);
        for (const l of loaders) expect(deps).toContain(l);
        // And the engine and wasm that the 5:51 reload also had to optimize.
        expect(deps).toContain("shiki/engine/oniguruma");
        expect(deps).toContain("shiki/wasm");
    });
});
