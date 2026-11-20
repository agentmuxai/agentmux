// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette's surface and stacking (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R24, R27): over a magnified or
 * full-screen pane; opaque, on the menu family's surface; and readable on every built-in theme and in High Contrast:
 * the label, the detail, the group name and the key caps at 4.5:1 on the panel and on the selected row, and the
 * selected row marked by more than a colour in High Contrast.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { bundledThemeIds, contrast, resolveTheme } from "@/app/element/ui/theme-test-utils";

const app = (rel: string) => readFileSync(join(__dirname, "..", rel), "utf8");
const css = readFileSync(join(__dirname, "command-palette.scss"), "utf8");

describe("the palette's stacking", () => {
    const tokens = app("theme.scss");
    const z = (name: string) => Number(tokens.match(new RegExp(`--${name}:\\s*(\\d+)`))?.[1]);

    it("is above a magnified pane, the pane chrome and a dialog inside a pane", () => {
        const modal = z("z-modal");
        expect(modal).toBeGreaterThan(0);
        for (const name of ["z-layout-magnify", "z-layout-ephemeral", "z-layout-magnified-backdrop", "z-pane-modal", "z-pane-progress"]) {
            expect(z(name), `--${name} resolves`).toBeGreaterThan(0);
            expect(modal, `--z-modal is above --${name}`).toBeGreaterThan(z(name));
        }
    });
});

describe("the palette's surface", () => {
    it("stands on the menu family's opaque surface, not the pane's translucent one", () => {
        expect(css).toMatch(/@include menu-frame\.menu-surface;/);
    });

    it("marks the selected row by an outline in High Contrast", () => {
        expect(css).toMatch(/\[data-theme="high-contrast"\] &\.selected \{[^}]*outline: 2px solid var\(--accent-color\)/);
    });
});

describe.each(["default", ...bundledThemeIds()])("the palette on %s", (id) => {
    const t = resolveTheme(id);
    const surface = t.pick("menu-bg-color")!;
    const selected = t.washOf("highlight-bg-color", surface);
    const hover = t.washOf("hover-bg-color", surface);
    const text = t.pick("main-text-color")!;
    const muted = t.pick("secondary-text-color")!;
    const cap = t.pick("block-bg-color")!;

    it("reads the label at 4.5:1 on the panel, the hovered row and the selected row", () => {
        expect(contrast(text, surface)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(text, hover)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(text, selected)).toBeGreaterThanOrEqual(4.5);
    });

    it("reads the detail and the group name at 4.5:1 on the panel, in muted text", () => {
        expect(contrast(muted, surface)).toBeGreaterThanOrEqual(4.5);
    });

    it("reads them in the main text colour on the selected row, where muted text does not reach 4.5:1 everywhere", () => {
        expect(css).toMatch(/&\.selected \.command-palette-item-detail,\s*&\.selected \.command-palette-item-group \{\s*color: var\(--main-text-color\);/);
        expect(contrast(text, selected)).toBeGreaterThanOrEqual(4.5);
    });

    it("reads a key cap's text at 4.5:1 on the cap", () => {
        expect(contrast(text, cap)).toBeGreaterThanOrEqual(4.5);
    });
});
