// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every Tag tone reads in every bundled theme. The tag's fill is its tone at a few percent over
 * the modal surface (opaque, because a ribbon or a count sits over a card's text), its text is the
 * main text colour, and its outline is the full tone colour (the neutral tone's is the secondary
 * text colour at 60% over the surface). WCAG AA: 4.5:1 for the text on the fill, 3:1 for the
 * outline against each surface a tag sits on: the window (`--main-bg-color`), a card (a faint
 * tint of the text colour over it) and a panel (`--modal-bg-color`).
 *
 * A theme whose tone colour falls short as an outline overrides only its `--*-edge` token
 * (theme.scss), so a terminal or status colour that reads the tone colour is not recoloured.
 * docs/specs/SPEC_UI_PANEL_PRIMITIVE_2026_10_29.md §9.
 */

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { TAG_NEUTRAL_EDGE_PCT, TAG_TONE_TOKENS, TAG_TONES, type TagTone } from "./tag-tokens";
import { bundledThemeIds, contrast, over, resolveTheme, type Rgb } from "./theme-test-utils";

const TEXT_AA = 4.5;
const OUTLINE_AA = 3;

const scss = readFileSync(new URL("./tag.scss", import.meta.url), "utf8");

/** The `--tag-fill-pct` and `--tag-edge` a tone's block in tag.scss paints. */
function toneBlock(tone: TagTone): string {
    const m = scss.match(new RegExp(`&--${tone}\\s*\\{([^}]*)\\}`));
    if (!m) throw new Error(`tag.scss has no &--${tone} block`);
    return m[1];
}

describe("tag.scss paints the numbers the contrast test measures", () => {
    it("the base fill share is the coloured tones'", () => {
        const base = scss.match(/--tag-fill-pct:\s*(\d+)%/)?.[1];
        for (const tone of TAG_TONES) {
            if (tone !== "neutral") expect(TAG_TONE_TOKENS[tone].fillPct, tone).toBe(Number(base));
        }
    });

    it("neutral's fill and outline shares", () => {
        const block = toneBlock("neutral");
        expect(Number(block.match(/--tag-fill-pct:\s*(\d+)%/)?.[1])).toBe(TAG_TONE_TOKENS.neutral.fillPct);
        expect(Number(block.match(/--secondary-text-color\)\s+(\d+)%/g)?.[0].match(/(\d+)%/)?.[1])).toBe(
            TAG_NEUTRAL_EDGE_PCT,
        );
    });

    it("each coloured tone reads its colour token and its edge token", () => {
        for (const tone of TAG_TONES) {
            const { tone: colour, edge } = TAG_TONE_TOKENS[tone];
            const block = toneBlock(tone);
            expect(block, tone).toContain(`--tag-tone: var(--${colour})`);
            if (edge) expect(block, tone).toContain(`--tag-edge: var(--${edge})`);
        }
    });

    it("the fill is mixed over the modal surface", () => {
        expect(scss).toContain("--tag-surface: var(--modal-bg-color)");
    });
});

describe("Tag colours", () => {
    const themes = ["default", ...bundledThemeIds()];

    for (const id of themes) {
        it(`read in ${id}`, () => {
            const theme = resolveTheme(id);
            const main = theme.pick("main-bg-color");
            const modal = theme.pick("modal-bg-color");
            const text = theme.pick("main-text-color");
            const secondary = theme.pick("secondary-text-color");
            expect(main && modal && text && secondary, "the theme's surfaces and text resolve").toBeTruthy();

            // The three surfaces a tag's outline is drawn against.
            const surfaces: Record<string, Rgb> = {
                window: main!,
                card: theme.washOf("tint-1", main!),
                panel: modal!,
            };

            // Every shortfall in the theme is collected, so one run names them all.
            const short: string[] = [];
            const check = (what: string, ratio: number, bar: number) => {
                if (ratio < bar) short.push(`${what}: ${ratio.toFixed(2)} < ${bar}`);
            };

            for (const tone of TAG_TONES) {
                const { tone: colourToken, edge: edgeToken, fillPct } = TAG_TONE_TOKENS[tone];
                const colour = theme.pick(colourToken);
                expect(colour, `--${colourToken} in ${id}`).toBeTruthy();

                const fill = over(colour!, modal!, fillPct / 100);
                check(`${tone} text on its fill`, contrast(text!, fill), TEXT_AA);

                const edge = edgeToken ? theme.pick(edgeToken) : over(secondary!, modal!, TAG_NEUTRAL_EDGE_PCT / 100);
                expect(edge, `the ${tone} outline resolves in ${id}`).toBeTruthy();
                for (const [where, surface] of Object.entries(surfaces)) {
                    check(`${tone} outline on the ${where} surface`, contrast(edge!, surface), OUTLINE_AA);
                }
            }
            expect(short, `Tag colours below the bar in ${id}`).toEqual([]);
        });
    }
});
