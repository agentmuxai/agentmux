// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's tag colours stay apart and readable in every bundled theme: the six seat colours and the seven model
 * colours (`--seat-*`, `--model-*`, theme.scss). Measured the way view/kanban/kanban-stage-colours.test.ts measures
 * its stages: each set's colours at least 9 apart in OKLab (x100), and in the way tag-contrast.test.ts measures a
 * tone: the outline 3:1 against every surface a tag sits on (the window, a card, a panel, and the selected row's
 * highlight over the window), the text 4.5:1 on the colour's wash, and a muted tag's quiet text (the secondary text
 * colour, on the bare surface) 4.5:1 on each of those surfaces. The two sets are not asked to be apart from
 * each other: they differ by shape, glyph and place. A theme where two meet, or an outline is short, overrides an
 * alias or an edge in its own file (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §6.3).
 */

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { TAG_HUE_FILL_PCT, TAG_HUES } from "./tag-tokens";
import { apartOklab, bundledThemeIds, contrast, over, resolveTheme, type Rgb } from "./theme-test-utils";

const MIN_APART = 9;
const TEXT_AA = 4.5;
const OUTLINE_AA = 3;

const scss = readFileSync(new URL("./tag.scss", import.meta.url), "utf8");

describe("tag.scss paints each hue from its tokens", () => {
    it("every hue has a block with its colour token and its edge token, at the coloured tones' fill", () => {
        const base = Number(scss.match(/--tag-fill-pct:\s*(\d+)%/)?.[1]);
        expect(TAG_HUE_FILL_PCT).toBe(base);
        const list = scss.match(/@each \$hue in ([^{]*)\{/)?.[1].split(",").map((n) => n.trim()) ?? [];
        expect(list.sort()).toEqual([...TAG_HUES.seat, ...TAG_HUES.model].sort());
        expect(scss).toContain("--tag-tone: var(--#{$hue})");
        expect(scss).toContain("--tag-edge: var(--#{$hue}-edge)");
    });
});

describe("Swarm tag colours", () => {
    const themes = ["default", ...bundledThemeIds()];

    it("are measured in every theme", () => {
        expect(themes.length).toBeGreaterThanOrEqual(13);
    });

    for (const id of themes) {
        it(`read and stay apart in ${id}`, () => {
            const theme = resolveTheme(id);
            const main = theme.pick("main-bg-color");
            const modal = theme.pick("modal-bg-color");
            const text = theme.pick("main-text-color");
            expect(main && modal && text, "the theme's surfaces and text resolve").toBeTruthy();
            const surfaces: Record<string, Rgb> = {
                window: main!,
                card: theme.washOf("tint-1", main!),
                panel: modal!,
                "selected row": theme.washOf("highlight-bg-color", main!),
            };

            const short: string[] = [];
            const colours = new Map<string, Rgb>();
            for (const hue of [...TAG_HUES.seat, ...TAG_HUES.model]) {
                const colour = theme.pick(hue);
                const edge = theme.pick(`${hue}-edge`);
                expect(colour, `--${hue} in ${id}`).toBeTruthy();
                expect(edge, `--${hue}-edge in ${id}`).toBeTruthy();
                colours.set(hue, colour!);
                const ratio = contrast(text!, over(colour!, modal!, TAG_HUE_FILL_PCT / 100));
                if (ratio < TEXT_AA) short.push(`${hue} text on its fill: ${ratio.toFixed(2)} < ${TEXT_AA}`);
                for (const [where, surface] of Object.entries(surfaces)) {
                    const edgeRatio = contrast(edge!, surface);
                    if (edgeRatio < OUTLINE_AA) short.push(`${hue} outline on the ${where}: ${edgeRatio.toFixed(2)} < ${OUTLINE_AA}`);
                }
            }
            // A muted tag (a model asked for, not confirmed) has no fill: its words are the secondary text colour on the bare
            // surface the row gives it, so that colour has to reach the text bar on each of them. Its dashed outline is the
            // hue's edge, measured against the same surfaces above.
            const secondary = theme.pick("secondary-text-color");
            expect(secondary, `--secondary-text-color in ${id}`).toBeTruthy();
            for (const [where, surface] of Object.entries(surfaces)) {
                const ratio = contrast(secondary!, surface);
                if (ratio < TEXT_AA) short.push(`muted text on the ${where}: ${ratio.toFixed(2)} < ${TEXT_AA}`);
            }
            for (const set of [TAG_HUES.seat, TAG_HUES.model]) {
                for (let i = 0; i < set.length; i++) {
                    for (let j = i + 1; j < set.length; j++) {
                        const d = apartOklab(colours.get(set[i])!, colours.get(set[j])!);
                        if (d < MIN_APART) short.push(`${set[i]} and ${set[j]} are ${d.toFixed(1)} apart, under ${MIN_APART}`);
                    }
                }
            }
            expect(short, `Swarm tag colours below the bar in ${id}`).toEqual([]);
        });
    }
});
