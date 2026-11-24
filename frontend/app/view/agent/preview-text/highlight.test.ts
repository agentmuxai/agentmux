// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A highlighted preview carries both themes' colours, so a light theme shows the light one: before, only the dark
 * theme was used and a Read or Edit preview on a light pane had near-white punctuation and names.
 */
import { describe, expect, it } from "vitest";
import { highlightDoc, SHIKI_THEMES } from "./highlight";
import type { PreviewDoc } from "./types";

const doc = (kind: "code" | "diff", lines: PreviewDoc["lines"]): PreviewDoc => ({ kind, lang: "rust", lines });

describe("highlighting a preview", () => {
    it("names a dark and a light Shiki theme", () => {
        expect(SHIKI_THEMES.dark).toMatch(/dark/);
        expect(SHIKI_THEMES.light).toMatch(/light/);
    });

    it("gives every coloured run both themes' colours as CSS variables, and nothing else", async () => {
        const tokens = await highlightDoc(doc("code", [{ text: "fn main() {" }, { text: "    let v: Vec<u8> = Vec::new();" }, { text: "}" }]));
        expect(tokens).not.toBeNull();
        const runs = tokens!.flatMap((line) => line ?? []);
        const coloured = runs.filter((r) => r.vars);
        expect(coloured.length).toBeGreaterThan(3);
        for (const r of coloured) {
            expect(Object.keys(r.vars!).every((k) => k.startsWith("--shiki-"))).toBe(true);
            expect(r.vars!["--shiki-dark"]).toMatch(/^#/);
            expect(r.vars!["--shiki-light"]).toMatch(/^#/);
        }
        // The two are different palettes: somewhere the light colour is not the dark one.
        expect(coloured.some((r) => r.vars!["--shiki-light"].toLowerCase() !== r.vars!["--shiki-dark"].toLowerCase())).toBe(true);
    });

    it("keeps the text of the preview exactly", async () => {
        const lines = [{ text: "let a = 1;" }, { text: "let b = a + 2;" }];
        const tokens = await highlightDoc(doc("code", lines));
        expect(tokens!.map((l) => (l ?? []).map((r) => r.content).join(""))).toEqual(lines.map((l) => l.text));
    });

    it("highlights a diff's two sides in both themes too", async () => {
        const tokens = await highlightDoc(
            doc("diff", [
                { text: "let a = 1;", marker: "-" },
                { text: "let a = 2;", marker: "+" },
            ])
        );
        expect(tokens).toHaveLength(2);
        expect(tokens!.every((l) => (l ?? []).some((r) => r.vars?.["--shiki-light"]))).toBe(true);
    });
});
