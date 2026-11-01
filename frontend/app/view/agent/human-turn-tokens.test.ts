// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Only humans get the inverted colour" as a property the build checks
 * (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §4.3, S7): the human-turn tokens are used by
 * the human-turn stylesheet alone, and no other message stylesheet fills a block with the text colour.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const STYLES = join(__dirname, "styles");
const HUMAN = "_human-turn.scss";
const stripComments = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
const sheets = readdirSync(STYLES)
    .filter((f) => f.endsWith(".scss"))
    .map((f) => ({ name: f, css: stripComments(readFileSync(join(STYLES, f), "utf8")) }));

describe("human-turn tokens", () => {
    it("are used by the human-turn stylesheet, and by no other", () => {
        expect(sheets.find((s) => s.name === HUMAN)?.css).toContain("--human-turn-bg");
        const others = sheets.filter((s) => s.name !== HUMAN && /--human-turn-/.test(s.css)).map((s) => s.name);
        expect(others).toEqual([]);
    });

    it("are defined once, in the theme", () => {
        const theme = readFileSync(join(__dirname, "../../theme.scss"), "utf8");
        for (const token of ["--human-turn-bg", "--human-turn-fg", "--human-turn-edge"]) {
            expect(theme.match(new RegExp(`${token}:`, "g"))).toHaveLength(1);
        }
    });

    it("no other stylesheet fills a block with the text colour (the inverted fill)", () => {
        const offenders = sheets
            .filter((s) => s.name !== HUMAN && /background(-color)?:\s*var\(--main-text-color\)/.test(s.css))
            .map((s) => s.name);
        expect(offenders).toEqual([]);
    });

    it("the origin container never uses them", () => {
        const origin = sheets.find((s) => s.name === "_origin-container.scss")!;
        expect(origin.css).not.toMatch(/human-turn|user-input-color|block-bg-solid-color/);
    });
});
