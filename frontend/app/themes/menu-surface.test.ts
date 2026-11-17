// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The menu surface (`menu-surface` in element/menu-frame.scss, tokens `--menu-*`) stands out from the
 * pane on every theme: opaque at any window opacity, a step off the pane, an edge at 3:1 against it
 * (the WCAG 1.4.11 number), item text at 4.5:1, and every menu-family stylesheet on the one mixin
 * rather than its own background or shadow. Spec: docs/specs/SPEC_MENU_SURFACES_STAND_OUT_2026_11_17.md §5.1.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { bundledThemeIds, contrast, luminance, over, resolveTheme, type Rgb } from "@/app/element/ui/theme-test-utils";

const read = (rel: string) => readFileSync(new URL(`../${rel}`, import.meta.url), "utf8");

const WHITE: Rgb = [255, 255, 255];

describe.each(["default", ...bundledThemeIds()])("the menu surface on %s", (id) => {
    const t = resolveTheme(id);
    const pane = t.pick("main-bg-color")!;
    const surface = t.pick("menu-bg-color")!;
    const edge = t.pick("menu-edge-color")!;

    it("resolves every token", () => {
        expect(pane && surface && edge).toBeTruthy();
        expect(t.raw("menu-ring-color")).toMatch(/rgba\(0, 0, 0, 0\.\d+\)/);
        expect(t.raw("menu-shadow")).toBeTruthy();
    });

    it("is a step off the pane (lighter in a dark theme, white in a light one)", () => {
        if (t.light) expect(surface).toEqual(WHITE);
        else {
            expect(luminance(surface)).toBeGreaterThan(luminance(pane));
            expect(contrast(surface, pane)).toBeGreaterThanOrEqual(1.17);
        }
    });

    it("has an edge at 3:1 against the pane", () => {
        expect(contrast(edge, pane)).toBeGreaterThanOrEqual(3);
    });

    it("keeps the edge off the surface too, and off the terminal's own background", () => {
        expect(contrast(edge, surface)).toBeGreaterThanOrEqual(2.3);
        const term = t.pick("term-background");
        expect(term, "--term-background resolves").toBeTruthy();
        expect(contrast(edge, term!)).toBeGreaterThanOrEqual(2.3);
    });

    it("keeps item text at AA on the surface, on a hovered row and on a danger row", () => {
        const text = t.pick("main-text-color")!;
        expect(contrast(text, surface)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(text, t.washOf("hover-bg-color", surface))).toBeGreaterThanOrEqual(4.5);
        const danger = t.pick("menu-danger-text-color")!;
        expect(contrast(danger, surface)).toBeGreaterThanOrEqual(4.5);
        expect(contrast(danger, t.washOf("error-bg-strong", surface))).toBeGreaterThanOrEqual(4.5);
    });

    it("has a row separator you can see against the surface", () => {
        // The token is the edge mixed 55% into the surface (each with a fallback for a theme that sets neither).
        expect(t.raw("menu-separator-color")).toBe(
            "color-mix(in srgb, var(--menu-edge-color, var(--border-color)) 55%, var(--menu-bg-color, var(--modal-bg-color)))",
        );
        expect(contrast(over(edge, surface, 0.55), surface)).toBeGreaterThanOrEqual(1.4);
    });

    it("is opaque: a literal colour that never follows the window opacity", () => {
        expect(t.raw("menu-bg-color")).toMatch(/^\s*(#[0-9a-f]{6}|rgb\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\))\s*$/i);
        expect(t.raw("menu-edge-color")).toMatch(/^\s*#[0-9a-f]{6}\s*$/i);
    });
});

describe("the menu surface at any window opacity", () => {
    /** A token's alpha when `--window-opacity` is `opacity`: the number the cascade would paint with. */
    const alphaAt = (value: string | undefined, opacity: number): number => {
        const text = (value ?? "").replace(/var\(--window-opacity[^)]*\)/g, String(opacity));
        const rgba = text.match(/rgba\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*,\s*([\d.]+)\s*\)/);
        return rgba ? Number(rgba[1]) : 1;
    };

    for (const id of ["default", ...bundledThemeIds()]) {
        it(`looks the same at 0.3 and at 1 on ${id}, where the pane does not`, () => {
            const t = resolveTheme(id);
            // The control: the pane's own colour follows the opacity (it is why the old menu was see-through).
            expect(alphaAt(t.raw("main-bg-color"), 0.3)).toBeCloseTo(0.3);
            expect(alphaAt(t.raw("main-bg-color"), 1)).toBeCloseTo(1);
            // The menu's does not.
            expect(alphaAt(t.raw("menu-bg-color"), 0.3)).toBe(1);
            expect(alphaAt(t.raw("menu-bg-color"), 1)).toBe(1);
            expect(t.raw("menu-bg-color")).not.toMatch(/var\(/);
        });
    }

    it("is not built from the pane's colour, the window opacity or a translucent mix", () => {
        const mixin = read("element/menu-frame.scss").replace(/\/\/.*$/gm, "");
        expect(mixin).not.toMatch(/main-bg-color|window-opacity|transparent|backdrop-filter/);
        expect(mixin).toMatch(/var\(--menu-bg-color, var\(--modal-bg-color\)\)/);
    });
});

/** The text of the first rule whose selector is `selector` (at the start of a line), without nested rules. */
function ownDeclarations(scss: string, selector: string): string | null {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const start = scss.match(new RegExp(`^[ \\t]*${escaped}\\s*\\{`, "m"));
    if (!start || start.index === undefined) return null;
    let depth = 1;
    let own = "";
    for (let i = start.index + start[0].length; i < scss.length && depth > 0; i++) {
        const ch = scss[i];
        if (ch === "{") depth++;
        else if (ch === "}") depth--;
        else if (depth === 1) own += ch;
    }
    return own.replace(/\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, "");
}

/** Every surface of the menu family: the stylesheet and the rule that must stand on the mixin. */
const MENU_SURFACES: [file: string, selector: string][] = [
    ["element/flyoutmenu.scss", ".menu"],
    ["components/context-menu.scss", ".ctx-menu"],
    ["tab/tab.scss", ".tab-context-panel"],
    ["window/action-widgets.scss", ".action-widget-more-dropdown"],
    ["statusbar/_instance-panel.scss", ".instance-panel"],
    ["view/agent/styles/_slash.scss", ".slash-autocomplete"],
    ["view/agent/styles/_input-history-search.scss", ".input-history-search"],
    ["view/agent/styles/_btw.scss", ".btw-overlay"],
    ["view/agent/styles/_document-nodes.scss", ".agent-node-peek-overlay"],
    ["view/settings/settings-search-bar.scss", ".settings-search-results"],
    ["view/browser/browser-view.scss", ".browser-address-suggestions"],
    ["view/accounts/accounts-gallery.scss", ".accounts-chooser"],
    ["backdrop/backdrop-picker.scss", ".backdrop-picker"],
];

describe("the menu-family stylesheets use the one mixin", () => {
    it.each(MENU_SURFACES)("%s: %s includes the surface and sets no background or shadow of its own", (file, selector) => {
        const own = ownDeclarations(read(file), selector);
        expect(own, `${selector} is in ${file}`).not.toBeNull();
        expect(own).toMatch(/@include\s+[\w-]+\.menu-(surface|frame)\s*;/);
        expect(own).not.toMatch(/(^|[;\s])(background(-color)?|box-shadow|backdrop-filter)\s*:/);
    });

    it("the status-bar popover frame stands on it too", () => {
        const popover = read("element/popover-frame.scss");
        expect(ownDeclarations(popover, "@mixin popover-frame")).toMatch(/@include\s+[\w-]+\.menu-surface\s*;/);
        expect(ownDeclarations(popover, "@mixin popover-frame")).not.toMatch(/(^|[;\s])(background|box-shadow)\s*:/);
    });

    it("the scan itself finds a rule that sets its own background (sensitivity)", () => {
        const bad = ".menu { @include menu-frame.menu-surface; background: red; .x { color: red } }";
        expect(ownDeclarations(bad, ".menu")).toMatch(/background\s*:/);
        const nested = ".menu { @include menu-frame.menu-surface; .x { background: red } }";
        expect(ownDeclarations(nested, ".menu")).not.toMatch(/background/);
    });
});
