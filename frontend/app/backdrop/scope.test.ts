// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A pane's colour scope: the properties `scopeVars` makes from a derived backdrop, and the per-block registry. */

import { createRoot, createEffect } from "solid-js";
import { describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
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
