// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const picker = readFileSync(new URL("../styles/_picker.scss", import.meta.url), "utf8");
const theme = readFileSync(new URL("../../../theme.scss", import.meta.url), "utf8");

/** A `--space-*` token's pixels, from the theme. */
const space = (name: string): number => Number(new RegExp(`--${name}:\\s*(\\d+)px`).exec(theme)?.[1]);

/** The ribbon is a line of xs text with its icon and 1px edges: 20px, measured on the card at its narrowest. */
const RIBBON_PX = 20;

describe("a picker card's corner ribbon never sits on its caption", () => {
    it("reserves under the caption the ribbon's height and a gap, for a card with no chips", () => {
        const room = /--agent-card-ribbon-room:\s*calc\(([^;]+)\);/.exec(picker)?.[1] ?? "";
        const pixels = [...room.matchAll(/var\(--([\w-]+)\)/g)].reduce((sum, m) => sum + space(m[1]), 0);
        expect(pixels).toBeGreaterThanOrEqual(RIBBON_PX + 8);
    });

    it("applies it to a card with a corner action and no chips, at every width", () => {
        const rule = /&\.agent-card--has-action:not\(\.agent-card--has-tags\)\s*\{([^}]*)\}/.exec(picker)?.[1] ?? "";
        expect(rule).toMatch(/padding-bottom:\s*var\(--agent-card-ribbon-room\)/);
    });
});
