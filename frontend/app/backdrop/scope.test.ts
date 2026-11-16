// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A pane's colour scope: the properties `scopeVars` makes from a derived backdrop, and the per-block registry. */

import { createRoot, createEffect } from "solid-js";
import { describe, expect, it } from "vitest";
import { contrastRatio } from "../block/pane-color-scheme";
import { ANSI_TOKENS } from "./ansi";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { deriveInk } from "./ink";
import { oklchToHex } from "../theme/oklch";
import { getPaneScope, INK_TOKEN_MAP, MAPPED_TOKENS, registerPaneScope, releasePaneScope, scopeVars, SURFACE_TOKEN_MAP } from "./scope";

// Any built-in will do: the first, on each of the app's two themes.
const derived = deriveBuiltin(BUILTINS[0].id, "dark")!;

describe("scopeVars", () => {
    const vars = scopeVars(derived);

    it("sets the backdrop's own properties from the derived surfaces and ink", () => {
        expect(vars["color-scheme"]).toBe(derived.scheme);
        expect(vars["--surface"]).toBe(derived.surfaces.surface);
        expect(vars["--surface-raised"]).toBe(derived.surfaces.raised);
        expect(vars["--surface-overlay"]).toBe(derived.surfaces.overlay);
        expect(vars["--ink-text"]).toBe(derived.ink.text);
        expect(vars["--ink-muted"]).toBe(derived.ink.muted);
        expect(vars["--ink-faint"]).toBe(derived.ink.faint);
        expect(vars["--ink-border"]).toBe(derived.ink.border);
        expect(vars["--ink-divider"]).toBe(derived.ink.divider);
        expect(vars["--ink-accent"]).toBe(derived.ink.accent);
        expect(vars["--ink-hover"]).toBe(derived.ink.hover);
        expect(vars["--ink-selected"]).toBe(derived.ink.selected);
    });

    it("dims the two backgrounds with the window's opacity, as the theme's own do", () => {
        const dimmed = "color-mix(in srgb, var(--surface) calc(var(--window-opacity, 1) * 100%), transparent)";
        expect(vars["--main-bg-color"]).toBe(dimmed);
        expect(vars["--block-bg-color"]).toBe(dimmed);
        expect(vars["--elevated-bg-color"]).toBe("var(--surface-raised)");
    });

    it("points the text, border, accent and status tokens at the matching ink", () => {
        expect(vars["--main-text-color"]).toBe("var(--ink-text)");
        expect(vars["--secondary-text-color"]).toBe("var(--ink-muted)");
        expect(vars["--tertiary-text-color"]).toBe("var(--ink-faint)");
        expect(vars["--border-color"]).toBe("var(--ink-border)");
        expect(vars["--accent-color"]).toBe("var(--ink-accent)");
        expect(vars["--accent-fg-color"]).toBe("var(--ink-accent)");
        expect(vars["--link-color"]).toBe("var(--ink-accent)");
        for (const kind of ["error", "warning", "success"]) {
            expect(vars[`--${kind}-color`]).toBe(`var(--ink-${kind})`);
            expect(vars[`--${kind}-text-color`]).toBe(`var(--ink-${kind})`);
        }
    });

    it("re-points the fixed-colour tokens the agent pane paints with: panel, modal, hover, strip, solid block and grey", () => {
        expect(vars["--modal-bg-color"]).toBe("var(--surface-overlay)");
        expect(vars["--block-bg-solid-color"]).toBe("var(--surface)");
        expect(vars["--panel-bg-color"]).toBe("color-mix(in srgb, var(--surface-raised) 50%, transparent)");
        expect(vars["--tab-strip-bg"]).toBe("color-mix(in srgb, var(--ink-text) 3%, transparent)");
        expect(vars["--hover-bg-color"]).toBe("var(--ink-hover)");
        expect(vars["--grey-text-color"]).toBe("var(--ink-faint)");
    });

    it("gives --accent-color-rgb the accent's channels, which rgba(var(--accent-color-rgb), a) needs", () => {
        const n = parseInt(derived.ink.accent.slice(1, 7), 16);
        expect(vars["--accent-color-rgb"]).toBe(`${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`);
    });

    it("sets every token the map names, and every ink property a mapped token reads", () => {
        for (const token of MAPPED_TOKENS) expect(vars[token], token).toBeTruthy();
        for (const source of new Set(Object.values(INK_TOKEN_MAP))) expect(vars[source], source).toBeTruthy();
        expect(Object.keys(SURFACE_TOKEN_MAP)).toEqual([
            "--main-bg-color",
            "--block-bg-color",
            "--elevated-bg-color",
            "--modal-bg-color",
            "--panel-bg-color",
            "--block-bg-solid-color",
            "--tab-strip-bg",
        ]);
    });

    it("takes color-scheme from the side the surface is on", () => {
        const light = deriveBuiltin(BUILTINS[0].id, "light")!;
        expect(scopeVars(light)["color-scheme"]).toBe("light");
        expect(vars["color-scheme"]).toBe("dark");
    });
});

describe("scopeVars with the theme's terminal palette", () => {
    // The default theme's palette, as a stand-in for whatever theme is active.
    const palette = Object.fromEntries(
        ANSI_TOKENS.map((t, i) => [t, ["#3465a4", "#32afff", "#34e2e2", "#58c142", "#ad7fa8", "#06989a", "#4e9a06", "#bc3fbc", "#c4a000"][i]])
    );

    it("sets none of them without the palette, so they stay the theme's", () => {
        const vars = scopeVars(derived);
        for (const token of ANSI_TOKENS) expect(token in vars, token).toBe(false);
    });

    it("sets each one read to 4.5:1 on all three layers, on a pale and on a dark backdrop", () => {
        for (const [id, scheme] of [["yellow", "light"], ["indigo", "dark"]] as const) {
            const d = deriveBuiltin(id, scheme)!;
            const vars = scopeVars(d, palette);
            for (const token of ANSI_TOKENS) {
                for (const layer of Object.values(d.surfaces)) expect(contrastRatio(vars[token], layer), `${token} on ${id} ${layer}`).toBeGreaterThanOrEqual(4.5);
            }
        }
    });

    it("keeps a palette colour that already reads, as it is", () => {
        const dark = deriveBuiltin("indigo", "dark")!;
        expect(scopeVars(dark, { "--term-bright-cyan": "#34e2e2" })["--term-bright-cyan"]).toBe("#34e2e2");
    });

    it("moves one that doesn't: bright cyan on a pale backdrop", () => {
        const pale = deriveBuiltin("yellow", "light")!;
        const moved = scopeVars(pale, { "--term-bright-cyan": "#34e2e2" })["--term-bright-cyan"];
        expect(moved).not.toBe("#34e2e2");
    });
});

describe("the transcript's palette on a mid-tone custom backdrop", () => {
    it("holds 4.5:1 on all three layers, not only the one it started closest to (Latte's bright magenta on #5f5f5f)", () => {
        const d = deriveInk("#5f5f5f");
        const moved = scopeVars(d, { "--term-bright-magenta": "#8839ef" })["--term-bright-magenta"];
        for (const layer of Object.values(d.surfaces)) {
            expect(contrastRatio(moved, layer), `${moved} on ${layer}`).toBeGreaterThanOrEqual(4.5);
        }
    });

    it("holds across a sweep of mid-tone custom colours, for hues from the default and the Latte palettes", () => {
        const palette = ["#3465a4", "#32afff", "#34e2e2", "#58c142", "#ad7fa8", "#06989a", "#4e9a06", "#bc3fbc", "#c4a000", "#8839ef", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5"];
        const bad: string[] = [];
        for (let l = 0.3; l <= 0.75; l += 0.0125) {
            for (const c of [0, 0.03, 0.08]) {
                const d = deriveInk(oklchToHex(l, c, 280));
                const best = Math.max(...Object.values(d.surfaces).map((s) => Math.max(contrastRatio("#ffffff", s), contrastRatio("#000000", s))));
                if (best < 4.5 + 0.1) continue; // no lightness reads there at all
                for (const hex of palette) {
                    const moved = scopeVars(d, { "--term-red": hex })["--term-red"];
                    for (const layer of Object.values(d.surfaces)) {
                        if (contrastRatio(moved, layer) < 4.5) bad.push(`${d.chosen}: ${hex} -> ${moved} on ${layer}`);
                    }
                }
            }
        }
        expect(bad.slice(0, 8)).toEqual([]);
    });
});

describe("the pane scope registry", () => {
    it("is empty until a block registers, and readable before then", () => {
        expect(getPaneScope("block-never-registered")()).toBeUndefined();
    });

    it("holds each block's own vars", () => {
        registerPaneScope("block-a", { "--surface": "#111111" });
        registerPaneScope("block-b", { "--surface": "#eeeeee" });
        expect(getPaneScope("block-a")()).toEqual({ "--surface": "#111111" });
        expect(getPaneScope("block-b")()).toEqual({ "--surface": "#eeeeee" });
        releasePaneScope("block-a");
        releasePaneScope("block-b");
    });

    it("tells a reader that subscribed before the block registered, and again when it changes or clears", () => {
        const seen: Array<unknown> = [];
        const dispose = createRoot((d) => {
            const scope = getPaneScope("block-c");
            createEffect(() => seen.push(scope()));
            return d;
        });
        registerPaneScope("block-c", { "--surface": "#222222" });
        registerPaneScope("block-c", { "--surface": "#333333" });
        registerPaneScope("block-c", undefined);
        dispose();
        releasePaneScope("block-c");
        expect(seen).toEqual([undefined, { "--surface": "#222222" }, { "--surface": "#333333" }, undefined]);
    });

    it("forgets a released block", () => {
        registerPaneScope("block-d", { "--surface": "#444444" });
        releasePaneScope("block-d");
        expect(getPaneScope("block-d")()).toBeUndefined();
    });
});
